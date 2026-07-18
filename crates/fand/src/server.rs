//! Unix-socket server: one listener thread, one thread per connection.
//! Requests are newline-delimited JSON (fand-proto). `get_status` is
//! request/response; `subscribe_status` consumes the connection and pushes
//! one status per control tick until the client hangs up.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use fand_proto::{Command, Request, Response, ResponseData, PROTOCOL_VERSION};

use crate::hub::{EngineCommand, StatusHub};

/// How long a connection thread waits for the engine to answer a forwarded
/// command. Normal handling is sub-second; hitting this means the control
/// loop is wedged (e.g. blocked on hardware), which the client should hear
/// about rather than hang.
const ENGINE_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Removes the socket file on drop so a clean shutdown never leaves a stale
/// path behind (an unclean one is handled by `bind` unlinking first).
pub struct SocketCleanup(PathBuf);

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Bind the socket and fix ownership/permissions: mode 0660, group `fand`
/// when it exists — group members can talk to the daemon, nobody else can.
pub fn bind(path: &Path) -> Result<(UnixListener, SocketCleanup)> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // A SIGKILLed daemon leaves the file behind, and bind refuses to reuse it.
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    let cleanup = SocketCleanup(path.to_path_buf());

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))
        .with_context(|| format!("setting permissions on {}", path.display()))?;
    match fand_group_gid() {
        Some(gid) => {
            // Best effort: fails when not root (e.g. --dry-run with a
            // --socket in /tmp), where the owner-only default is fine.
            if let Err(e) = std::os::unix::fs::lchown(path, None, Some(gid)) {
                eprintln!("fand: could not set socket group: {e}");
            }
        }
        None => eprintln!("fand: group `fand` not found — socket usable by root only"),
    }
    eprintln!("fand: listening on {}", path.display());
    Ok((listener, cleanup))
}

/// Accept loop in its own thread, one more thread per connection. Client
/// errors (bad JSON, hangups) never propagate to the daemon.
pub fn spawn(listener: UnixListener, hub: Arc<StatusHub>, commands: mpsc::Sender<EngineCommand>) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let hub = Arc::clone(&hub);
                    let commands = commands.clone();
                    thread::spawn(move || {
                        let _ = handle_client(stream, &hub, &commands);
                    });
                }
                Err(e) => eprintln!("fand: accept failed: {e}"),
            }
        }
    });
}

fn handle_client(
    stream: UnixStream,
    hub: &StatusHub,
    commands: &mpsc::Sender<EngineCommand>,
) -> Result<()> {
    let mut writer = stream.try_clone().context("cloning stream")?;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                send(&mut writer, &Response::err(format!("bad request: {e}")))?;
                continue;
            }
        };
        if request.version != PROTOCOL_VERSION {
            send(
                &mut writer,
                &Response::err(format!(
                    "unsupported protocol version {} (daemon speaks {PROTOCOL_VERSION})",
                    request.version
                )),
            )?;
            continue;
        }
        match request.cmd {
            Command::GetStatus => {
                let response = match hub.latest() {
                    Some((_, status)) => Response::ok(ResponseData::Status(status)),
                    None => Response::err("no status yet (first tick pending)"),
                };
                send(&mut writer, &response)?;
            }
            Command::SubscribeStatus => return subscribe(&mut writer, hub),
            // Everything else needs engine state (config, overrides) and is
            // handled on the engine thread — the only place that may touch
            // hardware.
            cmd => send(&mut writer, &forward_to_engine(cmd, commands))?,
        }
    }
    Ok(())
}

/// Push the current snapshot, then every newer one, until a write fails
/// (client hung up). The 1 s wait quantum only bounds how long we sleep per
/// condvar round — pushes happen as soon as the control loop publishes.
fn subscribe(writer: &mut UnixStream, hub: &StatusHub) -> Result<()> {
    let mut last_seq = 0;
    if let Some((seq, status)) = hub.latest() {
        last_seq = seq;
        send(writer, &Response::ok(ResponseData::Status(status)))?;
    }
    loop {
        if let Some((seq, status)) = hub.wait_newer(last_seq, Duration::from_secs(1)) {
            last_seq = seq;
            send(writer, &Response::ok(ResponseData::Status(status)))?;
        }
    }
}

/// Rendezvous with the engine thread: send the command with a fresh reply
/// channel, then block (bounded) for the outcome.
fn forward_to_engine(cmd: Command, commands: &mpsc::Sender<EngineCommand>) -> Response {
    let (reply_tx, reply_rx) = mpsc::channel();
    if commands
        .send(EngineCommand {
            cmd,
            reply: reply_tx,
        })
        .is_err()
    {
        // Receiver dropped — the control loop is gone (shutting down).
        return Response::err("daemon is shutting down");
    }
    match reply_rx.recv_timeout(ENGINE_REPLY_TIMEOUT) {
        Ok(response) => response,
        Err(_) => Response::err("control loop did not respond in time"),
    }
}

fn send(writer: &mut UnixStream, response: &Response) -> Result<()> {
    let mut line = serde_json::to_string(response).context("encoding response")?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .context("writing response")
}

fn fand_group_gid() -> Option<u32> {
    let groups = std::fs::read_to_string("/etc/group").ok()?;
    for line in groups.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some("fand") {
            // Skip the password field; the third field is the gid.
            return fields.nth(1)?.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use fand_proto::{ChannelStatus, Status};
    use std::collections::BTreeMap;

    fn sample_status(cpu: f64) -> Status {
        Status {
            temps: BTreeMap::from([("cpu".to_string(), cpu)]),
            channels: BTreeMap::from([(
                "pwm2".to_string(),
                ChannelStatus {
                    rpm: 750,
                    current_pwm: 94,
                    target_pwm: 96,
                    mode: "curve".to_string(),
                    override_remaining_s: None,
                },
            )]),
            config_generation: 0,
        }
    }

    /// Bound server on a temp socket + a connected client. The returned
    /// receiver is the test's stand-in for the engine thread.
    fn server_and_client() -> (
        tempfile::TempDir,
        Arc<StatusHub>,
        BufReader<UnixStream>,
        mpsc::Receiver<EngineCommand>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fand.sock");
        let (listener, cleanup) = bind(&path).unwrap();
        // The TempDir handles file cleanup in tests.
        std::mem::forget(cleanup);
        let hub = Arc::new(StatusHub::default());
        let (cmd_tx, cmd_rx) = mpsc::channel();
        spawn(listener, Arc::clone(&hub), cmd_tx);
        let client = UnixStream::connect(&path).unwrap();
        (dir, hub, BufReader::new(client), cmd_rx)
    }

    fn send_line(client: &mut BufReader<UnixStream>, line: &str) {
        client
            .get_mut()
            .write_all(format!("{line}\n").as_bytes())
            .unwrap();
    }

    fn read_response(client: &mut BufReader<UnixStream>) -> Response {
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn get_status_returns_latest() {
        let (_dir, hub, mut client, _cmd_rx) = server_and_client();
        hub.publish(sample_status(54.5));
        send_line(&mut client, r#"{"version":1,"cmd":"get_status"}"#);
        let resp = read_response(&mut client);
        assert!(resp.ok);
        let Some(ResponseData::Status(status)) = resp.data else {
            panic!("expected status data, got {resp:?}");
        };
        assert_eq!(status.temps["cpu"], 54.5);
        assert_eq!(status.channels["pwm2"].rpm, 750);
    }

    #[test]
    fn get_status_before_first_tick_is_an_error() {
        let (_dir, _hub, mut client, _cmd_rx) = server_and_client();
        send_line(&mut client, r#"{"version":1,"cmd":"get_status"}"#);
        let resp = read_response(&mut client);
        assert!(!resp.ok);
        assert!(resp.error.unwrap().contains("no status yet"));
    }

    #[test]
    fn bad_json_gets_error_and_connection_survives() {
        let (_dir, hub, mut client, _cmd_rx) = server_and_client();
        hub.publish(sample_status(50.0));
        send_line(&mut client, "not json");
        assert!(!read_response(&mut client).ok);
        // Same connection still works afterwards.
        send_line(&mut client, r#"{"version":1,"cmd":"get_status"}"#);
        assert!(read_response(&mut client).ok);
    }

    #[test]
    fn wrong_version_is_rejected() {
        let (_dir, _hub, mut client, _cmd_rx) = server_and_client();
        send_line(&mut client, r#"{"version":99,"cmd":"get_status"}"#);
        let resp = read_response(&mut client);
        assert!(!resp.ok);
        assert!(resp.error.unwrap().contains("version 99"));
    }

    #[test]
    fn engine_commands_are_forwarded_and_replied() {
        let (_dir, _hub, mut client, cmd_rx) = server_and_client();
        // Stub engine thread: answer one GetConfig with a fixed payload.
        let stub = thread::spawn(move || {
            let cmd = cmd_rx.recv().unwrap();
            assert_eq!(cmd.cmd, Command::GetConfig);
            cmd.reply
                .send(Response::ok(ResponseData::Config {
                    toml: "[daemon]\n".into(),
                    generation: 0,
                }))
                .unwrap();
        });
        send_line(&mut client, r#"{"version":1,"cmd":"get_config"}"#);
        let resp = read_response(&mut client);
        assert!(resp.ok, "{resp:?}");
        let Some(ResponseData::Config { toml, .. }) = resp.data else {
            panic!("expected config data, got {resp:?}");
        };
        assert_eq!(toml, "[daemon]\n");
        stub.join().unwrap();
    }

    #[test]
    fn engine_gone_yields_shutdown_error() {
        let (_dir, _hub, mut client, cmd_rx) = server_and_client();
        drop(cmd_rx);
        send_line(&mut client, r#"{"version":1,"cmd":"reload_config"}"#);
        let resp = read_response(&mut client);
        assert!(!resp.ok);
        assert!(resp.error.unwrap().contains("shutting down"));
    }

    #[test]
    fn subscribe_pushes_each_publish() {
        let (_dir, hub, mut client, _cmd_rx) = server_and_client();
        hub.publish(sample_status(50.0));
        send_line(&mut client, r#"{"version":1,"cmd":"subscribe_status"}"#);
        // Initial snapshot on subscribe...
        let first = read_response(&mut client);
        let Some(ResponseData::Status(s)) = first.data else {
            panic!("expected status");
        };
        assert_eq!(s.temps["cpu"], 50.0);
        // ...then one push per publish.
        hub.publish(sample_status(61.0));
        let Some(ResponseData::Status(s)) = read_response(&mut client).data else {
            panic!("expected status");
        };
        assert_eq!(s.temps["cpu"], 61.0);
    }

    #[test]
    fn stale_socket_file_is_replaced_on_bind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fand.sock");
        std::fs::write(&path, "stale").unwrap();
        let (_listener, _cleanup) = bind(&path).unwrap();
        drop(_cleanup);
        assert!(!path.exists(), "cleanup drop must remove the socket");
    }
}
