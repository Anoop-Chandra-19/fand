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
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(()); // clean EOF
        }
        // Only newline-terminated requests execute. `read_line` hands back
        // an EOF-truncated final record without its newline — but a client
        // whose send died mid-request reports that as a plain failure
        // ("known not-applied"), so executing the record anyway would make
        // that report a lie. Discarding it keeps the client's guarantee
        // true: nothing runs unless the send completed.
        if !line.ends_with('\n') {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        // Check the version *before* deserializing the full command: a
        // foreign-version request likely has a different command shape
        // (e.g. v1 SetConfig lacks the mandatory `expected`), and it must
        // get the clear version diagnostic, not "bad request".
        #[derive(serde::Deserialize)]
        struct VersionProbe {
            version: u32,
        }
        let version = serde_json::from_str::<VersionProbe>(&line)
            .map(|p| p.version)
            .unwrap_or(PROTOCOL_VERSION);
        if version != PROTOCOL_VERSION {
            send(
                &mut writer,
                &Response::err(format!(
                    "unsupported protocol version {version} (daemon speaks {PROTOCOL_VERSION})"
                )),
            )?;
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                send(&mut writer, &Response::err(format!("bad request: {e}")))?;
                continue;
            }
        };
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
    forward_to_engine_within(cmd, commands, ENGINE_REPLY_TIMEOUT)
}

fn forward_to_engine_within(
    cmd: Command,
    commands: &mpsc::Sender<EngineCommand>,
    timeout: Duration,
) -> Response {
    let mutating = may_mutate(&cmd);
    let (reply_tx, reply_rx) = mpsc::channel();
    if commands
        .send(EngineCommand {
            cmd,
            reply: reply_tx,
        })
        .is_err()
    {
        // Receiver dropped — the control loop is gone (shutting down).
        // The command was never received: known not-applied.
        return Response::err("daemon is shutting down");
    }
    match reply_rx.recv_timeout(timeout) {
        Ok(response) => response,
        // The command is already queued on the engine's channel and may
        // still be executed after this reply. For a mutation that makes the
        // outcome genuinely unknown, and the structured code tells clients
        // to say so; a read has no outcome to be unknown — it just failed.
        Err(_) if mutating => Response::err_outcome_unknown(
            "control loop did not respond in time (the command may still apply)",
        ),
        Err(_) => Response::err("control loop did not respond in time"),
    }
}

/// Whether a timed-out `cmd` could have changed daemon state. Reads are
/// listed explicitly so any future command defaults to mutating — the
/// safe direction is over-warning, never misreporting an applied change.
fn may_mutate(cmd: &Command) -> bool {
    !matches!(
        cmd,
        Command::GetStatus | Command::SubscribeStatus | Command::GetConfig
    )
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
            instance: 0,
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
        send_line(&mut client, r#"{"version":2,"cmd":"get_status"}"#);
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
        send_line(&mut client, r#"{"version":2,"cmd":"get_status"}"#);
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
        send_line(&mut client, r#"{"version":2,"cmd":"get_status"}"#);
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

    /// A v1 SetConfig doesn't even deserialize as a v2 command (no
    /// `expected`) — it must still get the version diagnostic, not
    /// "bad request".
    #[test]
    fn v1_set_config_gets_version_diagnostic_not_bad_request() {
        let (_dir, _hub, mut client, _cmd_rx) = server_and_client();
        send_line(
            &mut client,
            r#"{"version":1,"cmd":"set_config","toml":"[daemon]\n"}"#,
        );
        let resp = read_response(&mut client);
        assert!(!resp.ok);
        let error = resp.error.unwrap();
        assert!(error.contains("unsupported protocol version 1"), "{error}");
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
                    instance: 0,
                }))
                .unwrap();
        });
        send_line(&mut client, r#"{"version":2,"cmd":"get_config"}"#);
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
        send_line(&mut client, r#"{"version":2,"cmd":"reload_config"}"#);
        let resp = read_response(&mut client);
        assert!(!resp.ok);
        assert!(resp.error.unwrap().contains("shutting down"));
    }

    /// A complete JSON request that hits EOF without its trailing newline
    /// must never execute: the client's send died mid-request and will
    /// report "known not-applied" — executing the record would make that
    /// a lie. Runs `handle_client` directly and joins it, so "nothing
    /// reached the engine" is proof by construction, not a timeout race.
    #[test]
    fn eof_truncated_request_is_never_executed() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let hub = Arc::new(StatusHub::default());
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let handler = thread::spawn(move || handle_client(server, &hub, &cmd_tx));
        client
            .write_all(br#"{"version":2,"cmd":"reload_config"}"#)
            .unwrap();
        // End the stream without ever sending the newline.
        client.shutdown(std::net::Shutdown::Write).unwrap();
        handler.join().unwrap().unwrap();
        // The handler has returned (and dropped its sender): no command
        // can ever arrive after this point.
        assert!(
            cmd_rx.try_recv().is_err(),
            "truncated request reached the engine"
        );
    }

    /// Discarding an unterminated tail is surgical: a complete request
    /// pipelined before it still executes.
    #[test]
    fn pipelined_complete_then_truncated_executes_only_the_complete_one() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let hub = Arc::new(StatusHub::default());
        let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>();
        let handler = thread::spawn(move || handle_client(server, &hub, &cmd_tx));
        // Stub engine: reply to everything forwarded, record what arrived.
        let engine = thread::spawn(move || {
            let mut seen = Vec::new();
            while let Ok(EngineCommand { cmd, reply }) = cmd_rx.recv() {
                reply.send(Response::ok_empty()).unwrap();
                seen.push(cmd);
            }
            seen
        });
        client
            .write_all(
                b"{\"version\":2,\"cmd\":\"reload_config\"}\n{\"version\":2,\"cmd\":\"reload_config\"}",
            )
            .unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        // The complete first request gets its answer...
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let resp: Response = serde_json::from_str(&line).unwrap();
        assert!(resp.ok, "{resp:?}");
        handler.join().unwrap().unwrap();
        // ...and once the handler has returned, only that request has
        // ever reached the engine.
        assert_eq!(engine.join().unwrap(), vec![Command::ReloadConfig]);
    }

    /// A timed-out read has no outcome to be unknown — it must come back
    /// as a plain error, without the outcome-unknown code that would make
    /// clients say "may or may not have applied" about a GetConfig.
    #[test]
    fn get_config_timeout_is_plain_error_not_outcome_unknown() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>();
        let resp = forward_to_engine_within(Command::GetConfig, &cmd_tx, Duration::from_millis(10));
        drop(cmd_rx);
        assert!(!resp.ok);
        assert_eq!(resp.code, None);
        assert!(resp.error.unwrap().contains("did not respond"));
    }

    /// A reply timeout is not a refusal: the queued command may still run,
    /// and the response must carry the structured outcome-unknown code so
    /// clients never report it as a plain failure.
    #[test]
    fn engine_reply_timeout_is_outcome_unknown() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>();
        // Stub engine that receives the command but never answers.
        let resp =
            forward_to_engine_within(Command::ReloadConfig, &cmd_tx, Duration::from_millis(10));
        drop(cmd_rx);
        assert!(!resp.ok);
        assert_eq!(resp.code, Some(fand_proto::ErrorCode::OutcomeUnknown));
        assert!(resp.error.unwrap().contains("may still apply"));
    }

    #[test]
    fn subscribe_pushes_each_publish() {
        let (_dir, hub, mut client, _cmd_rx) = server_and_client();
        hub.publish(sample_status(50.0));
        send_line(&mut client, r#"{"version":2,"cmd":"subscribe_status"}"#);
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
