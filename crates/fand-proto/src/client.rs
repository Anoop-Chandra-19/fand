//! Blocking socket client shared by fanctl and the GUI backend.
//!
//! One [`Client`] per connection; the daemon serves any number of
//! request/response pairs on it. [`Client::subscribe`] consumes the client
//! and turns the connection into a status stream (one frame per daemon
//! tick) — the wire protocol has no way back to request/response after
//! that, which is why the type system takes the client away.

use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::{
    Command, ConfigVersion, ErrorCode, Request, Response, ResponseData, SetConfigResult, Status,
    PROTOCOL_VERSION,
};

#[derive(Debug)]
pub enum ClientError {
    /// Socket-level failure (send/receive).
    Io(std::io::Error),
    /// The daemon closed the connection.
    Disconnected,
    /// The daemon sent something we could not interpret.
    Protocol(String),
    /// The daemon understood the request and refused it.
    Daemon(String),
    /// The peer speaks a different protocol version — a known, clean
    /// refusal (nothing was applied), kept distinct from `Protocol` so
    /// mutating requests never blur it into "outcome unknown".
    VersionMismatch(String),
    /// The request may or may not have taken effect — either the daemon
    /// said so ([`ErrorCode::OutcomeUnknown`]), or the connection failed
    /// *after* a mutating command was sent. Callers must report this as
    /// "may or may not have applied", never as a plain failure.
    OutcomeUnknown(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "{e}"),
            ClientError::Disconnected => write!(f, "connection closed by daemon"),
            ClientError::Protocol(msg) => write!(f, "{msg}"),
            ClientError::Daemon(msg) => write!(f, "daemon: {msg}"),
            ClientError::VersionMismatch(msg) => write!(f, "{msg}"),
            ClientError::OutcomeUnknown(cause) => {
                write!(f, "outcome unknown ({cause})")
            }
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Io(e) => Some(e),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;

pub struct Client {
    reader: BufReader<UnixStream>,
}

impl Client {
    /// Returns the raw io::Error so callers can match on `ErrorKind`
    /// (fanctl turns NotFound/PermissionDenied into setup hints).
    pub fn connect(socket: impl AsRef<Path>) -> std::io::Result<Self> {
        let stream = UnixStream::connect(socket)?;
        Ok(Self {
            reader: BufReader::new(stream),
        })
    }

    /// Like [`Client::connect`], but every read and write on the socket
    /// fails after `timeout` instead of blocking forever. For callers that
    /// must stay responsive when the daemon accepts but never answers
    /// (the GUI backend); interactive callers like fanctl can keep the
    /// untimed `connect` — the user's Ctrl-C is their timeout.
    pub fn connect_with_timeout(
        socket: impl AsRef<Path>,
        timeout: Duration,
    ) -> std::io::Result<Self> {
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(Self {
            reader: BufReader::new(stream),
        })
    }

    /// Send one command and read one response. A daemon-side refusal
    /// (`ok: false`) comes back as [`ClientError::Daemon`].
    pub fn request(&mut self, cmd: Command) -> Result<Response> {
        self.send(cmd)?;
        self.read_response()
    }

    pub fn get_status(&mut self) -> Result<Status> {
        match self.request(Command::GetStatus)?.data {
            Some(ResponseData::Status(status)) => Ok(status),
            other => Err(unexpected_payload(other)),
        }
    }

    /// Current applied config as TOML text, plus which daemon instance and
    /// config generation it corresponds to (see `Status::instance`).
    pub fn get_config(&mut self) -> Result<ConfigSnapshot> {
        match self.request(Command::GetConfig)?.data {
            Some(ResponseData::Config {
                toml,
                generation,
                instance,
            }) => Ok(ConfigSnapshot {
                toml,
                version: ConfigVersion {
                    instance,
                    generation,
                },
            }),
            other => Err(unexpected_payload(other)),
        }
    }

    /// Like [`Client::request`], but for commands that mutate daemon
    /// state (SetConfig, ReloadConfig, Set/ClearOverride): once the
    /// command has been sent, any *unintelligible* result — lost
    /// connection, timeout, garbage — maps to
    /// [`ClientError::OutcomeUnknown`], because the daemon may have
    /// received and applied the command without us seeing the answer.
    /// Explicit refusals stay ordinary errors: they are known
    /// not-applied. (A send failure is too: the daemon executes only
    /// newline-terminated requests and discards an EOF-truncated record,
    /// so a request whose send did not complete never runs.)
    pub fn request_mutating(&mut self, cmd: Command) -> Result<Response> {
        self.send(cmd)?;
        match self.read_response() {
            Ok(response) => Ok(response),
            Err(
                e @ (ClientError::Daemon(_)
                | ClientError::VersionMismatch(_)
                | ClientError::OutcomeUnknown(_)),
            ) => Err(e),
            Err(e) => Err(ClientError::OutcomeUnknown(e.to_string())),
        }
    }

    /// Compare-and-set config write: applies `toml` only if the daemon is
    /// still at `expected`. Returns the daemon's structured outcome —
    /// including refusals ([`SetConfigResult::Conflict`] / `Rejected`),
    /// which are *outcomes*, not errors. Post-send failures map to
    /// [`ClientError::OutcomeUnknown`] (see [`Client::request_mutating`]).
    pub fn set_config(&mut self, toml: String, expected: ConfigVersion) -> Result<SetConfigResult> {
        match self
            .request_mutating(Command::SetConfig { toml, expected })?
            .data
        {
            Some(ResponseData::SetConfig(result)) => Ok(result),
            // An ok answer whose payload we can't interpret still means
            // the daemon processed *something* — the mutation may have
            // applied, so this is outcome-unknown, not a plain protocol
            // error.
            other => Err(ClientError::OutcomeUnknown(format!(
                "daemon sent unexpected payload: {other:?}"
            ))),
        }
    }

    /// Switch the connection to push mode: one status frame per daemon
    /// tick until either side closes.
    pub fn subscribe(mut self) -> Result<StatusStream> {
        self.send(Command::SubscribeStatus)?;
        Ok(StatusStream {
            client: self,
            done: false,
        })
    }

    fn send(&mut self, cmd: Command) -> Result<()> {
        let mut line = serde_json::to_string(&Request::new(cmd))
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        line.push('\n');
        self.reader
            .get_mut()
            .write_all(line.as_bytes())
            .map_err(ClientError::Io)
    }

    fn read_response(&mut self) -> Result<Response> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).map_err(ClientError::Io)?;
        if n == 0 {
            return Err(ClientError::Disconnected);
        }
        let response: Response = serde_json::from_str(&line)
            .map_err(|_| ClientError::Protocol(format!("bad response: {}", line.trim())))?;
        if response.version != PROTOCOL_VERSION {
            return Err(ClientError::VersionMismatch(format!(
                "daemon speaks protocol version {} but this tool speaks {} — \
                 fand, fanctl and the GUI must be installed together",
                response.version, PROTOCOL_VERSION
            )));
        }
        if !response.ok {
            let message = response
                .error
                .unwrap_or_else(|| "unknown error".to_string());
            return Err(match response.code {
                Some(ErrorCode::OutcomeUnknown) => ClientError::OutcomeUnknown(message),
                None => ClientError::Daemon(message),
            });
        }
        Ok(response)
    }
}

fn unexpected_payload(data: Option<ResponseData>) -> ClientError {
    ClientError::Protocol(format!("daemon sent unexpected payload: {data:?}"))
}

/// What [`Client::get_config`] returns: the applied config text and the
/// version identifying exactly which config state it is — the pair a
/// read-modify-write must hand back to [`Client::set_config`] as
/// `expected`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigSnapshot {
    pub toml: String,
    pub version: ConfigVersion,
}

/// Iterator over pushed status frames. Yields `Err(Disconnected)` exactly
/// once when the daemon goes away, then `None` — so `for frame in stream`
/// with `frame?` surfaces the disconnect instead of spinning on it.
pub struct StatusStream {
    client: Client,
    done: bool,
}

impl StatusStream {
    /// Adjust the read deadline mid-stream. The GUI pump derives its
    /// wedge-detection timeout from the daemon's configured tick interval,
    /// which it only learns (and which can change) after subscribing.
    pub fn set_read_timeout(&self, timeout: Duration) -> std::io::Result<()> {
        self.client.reader.get_ref().set_read_timeout(Some(timeout))
    }
}

impl Iterator for StatusStream {
    type Item = Result<Status>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.client.read_response() {
            Ok(response) => match response.data {
                Some(ResponseData::Status(status)) => Some(Ok(status)),
                other => Some(Err(unexpected_payload(other))),
            },
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    use crate::ChannelStatus;

    fn sample_status() -> Status {
        Status {
            temps: BTreeMap::from([("cpu".into(), 50.0)]),
            channels: BTreeMap::from([(
                "pwm2".into(),
                ChannelStatus {
                    rpm: 800,
                    current_pwm: 100,
                    target_pwm: 100,
                    mode: "curve".into(),
                    override_remaining_s: None,
                },
            )]),
            config_generation: 0,
            instance: 0,
        }
    }

    /// Stub daemon: accepts one connection, serves exactly one request
    /// (possibly with several response lines, as subscribe does), then
    /// closes — so tests can also observe the disconnect.
    fn stub_server(
        respond: impl FnOnce(Request) -> Vec<Response> + Send + 'static,
    ) -> (PathBuf, std::thread::JoinHandle<()>) {
        let dir = std::env::temp_dir().join(format!(
            "fand-proto-client-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut stream = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            for response in respond(request) {
                let mut out = serde_json::to_string(&response).unwrap();
                out.push('\n');
                stream.write_all(out.as_bytes()).unwrap();
            }
        });
        (socket, handle)
    }

    #[test]
    fn get_status_round_trips() {
        let (socket, server) = stub_server(|req| {
            assert_eq!(req.cmd, Command::GetStatus);
            vec![Response::ok(ResponseData::Status(sample_status()))]
        });
        let mut client = Client::connect(&socket).unwrap();
        assert_eq!(client.get_status().unwrap(), sample_status());
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn daemon_refusal_becomes_daemon_error() {
        let (socket, server) = stub_server(|_| vec![Response::err("nope")]);
        let mut client = Client::connect(&socket).unwrap();
        match client.request(Command::ReloadConfig) {
            Err(ClientError::Daemon(msg)) => assert_eq!(msg, "nope"),
            other => panic!("expected Daemon error, got {other:?}"),
        }
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn garbage_becomes_protocol_error() {
        // Hand-rolled stub: reads the request, replies with a non-JSON
        // line, closes.
        let dir = std::env::temp_dir().join(format!("fand-proto-garbage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut stream = stream;
            stream.write_all(b"not json\n").unwrap();
        });
        let mut client = Client::connect(&socket).unwrap();
        match client.request(Command::GetStatus) {
            Err(ClientError::Protocol(msg)) => assert!(msg.contains("not json"), "{msg}"),
            other => panic!("expected Protocol error, got {other:?}"),
        }
        server.join().unwrap();
    }

    #[test]
    fn eof_mid_request_is_disconnected() {
        let dir = std::env::temp_dir().join(format!("fand-proto-eof-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            // Drop the connection without answering.
        });
        let mut client = Client::connect(&socket).unwrap();
        match client.request(Command::GetStatus) {
            Err(ClientError::Disconnected) => {}
            other => panic!("expected Disconnected, got {other:?}"),
        }
        server.join().unwrap();
    }

    #[test]
    fn timed_out_request_is_io_error() {
        // Stub that accepts the connection and then never answers — the
        // wedged-daemon case connect_with_timeout exists for.
        let dir = std::env::temp_dir().join(format!("fand-proto-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("sock");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            // Hold the connection open, saying nothing, until the client
            // gives up and drops its end.
            let _ = reader.read_line(&mut line);
        });
        let mut client = Client::connect_with_timeout(&socket, Duration::from_millis(100)).unwrap();
        match client.request(Command::GetStatus) {
            Err(ClientError::Io(e)) => assert!(
                matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ),
                "unexpected kind: {e:?}"
            ),
            other => panic!("expected Io timeout, got {other:?}"),
        }
        drop(client);
        server.join().unwrap();
    }

    fn sample_expected() -> ConfigVersion {
        ConfigVersion {
            instance: 42,
            generation: 7,
        }
    }

    #[test]
    fn set_config_returns_structured_outcome() {
        let (socket, server) = stub_server(|req| {
            match req.cmd {
                Command::SetConfig { toml, expected } => {
                    assert_eq!(toml, "[daemon]\n");
                    assert_eq!(expected, sample_expected());
                }
                other => panic!("expected SetConfig, got {other:?}"),
            }
            vec![Response::ok(ResponseData::SetConfig(
                SetConfigResult::Conflict {
                    current: ConfigVersion {
                        instance: 42,
                        generation: 9,
                    },
                },
            ))]
        });
        let mut client = Client::connect(&socket).unwrap();
        let result = client
            .set_config("[daemon]\n".into(), sample_expected())
            .unwrap();
        assert_eq!(
            result,
            SetConfigResult::Conflict {
                current: ConfigVersion {
                    instance: 42,
                    generation: 9,
                },
            }
        );
        drop(client);
        server.join().unwrap();
    }

    /// Once a SetConfig went out, a dead or silent connection means the
    /// daemon may have applied it — that must surface as OutcomeUnknown,
    /// not as an ordinary failure.
    #[test]
    fn set_config_with_no_answer_is_outcome_unknown() {
        // Server reads the request and closes without answering.
        let (socket, server) = stub_server(|_| vec![]);
        let mut client = Client::connect(&socket).unwrap();
        match client.set_config("[daemon]\n".into(), sample_expected()) {
            Err(ClientError::OutcomeUnknown(_)) => {}
            other => panic!("expected OutcomeUnknown, got {other:?}"),
        }
        server.join().unwrap();
    }

    /// The daemon's structured outcome-unknown code (engine reply timeout)
    /// maps to the same variant — for any request kind.
    #[test]
    fn outcome_unknown_code_maps_to_variant() {
        let (socket, server) =
            stub_server(|_| vec![Response::err_outcome_unknown("control loop busy")]);
        let mut client = Client::connect(&socket).unwrap();
        match client.request(Command::ReloadConfig) {
            Err(ClientError::OutcomeUnknown(msg)) => assert_eq!(msg, "control loop busy"),
            other => panic!("expected OutcomeUnknown, got {other:?}"),
        }
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn response_version_mismatch_is_a_clear_error() {
        let (socket, server) = stub_server(|_| {
            vec![Response {
                version: 1,
                ..Response::ok_empty()
            }]
        });
        let mut client = Client::connect(&socket).unwrap();
        match client.request(Command::GetConfig) {
            Err(ClientError::VersionMismatch(msg)) => {
                assert!(msg.contains("protocol version 1"), "{msg}");
                assert!(msg.contains("installed together"), "{msg}");
            }
            other => panic!("expected VersionMismatch error, got {other:?}"),
        }
        drop(client);
        server.join().unwrap();
    }

    /// Mutating requests must never report a lost answer as a plain
    /// failure — the daemon may have applied the command.
    #[test]
    fn mutating_request_with_no_answer_is_outcome_unknown() {
        let (socket, server) = stub_server(|_| vec![]);
        let mut client = Client::connect(&socket).unwrap();
        match client.request_mutating(Command::ReloadConfig) {
            Err(ClientError::OutcomeUnknown(_)) => {}
            other => panic!("expected OutcomeUnknown, got {other:?}"),
        }
        server.join().unwrap();
    }

    /// ...but a version-mismatch answer is a known clean refusal and must
    /// keep its clear diagnostic even on the mutating path.
    #[test]
    fn mutating_request_version_mismatch_stays_a_clean_refusal() {
        let (socket, server) = stub_server(|_| {
            vec![Response {
                version: 1,
                ..Response::err("bad request")
            }]
        });
        let mut client = Client::connect(&socket).unwrap();
        match client.request_mutating(Command::ReloadConfig) {
            Err(ClientError::VersionMismatch(msg)) => {
                assert!(msg.contains("protocol version 1"), "{msg}")
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
        drop(client);
        server.join().unwrap();
    }

    /// An ok SetConfig response with an uninterpretable payload means the
    /// daemon processed something — outcome unknown, not a protocol error.
    #[test]
    fn set_config_with_wrong_payload_is_outcome_unknown() {
        let (socket, server) = stub_server(|_| vec![Response::ok_empty()]);
        let mut client = Client::connect(&socket).unwrap();
        match client.set_config("[daemon]\n".into(), sample_expected()) {
            Err(ClientError::OutcomeUnknown(msg)) => {
                assert!(msg.contains("unexpected payload"), "{msg}")
            }
            other => panic!("expected OutcomeUnknown, got {other:?}"),
        }
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn subscribe_yields_frames_then_one_disconnect() {
        let (socket, server) = stub_server(|req| {
            assert_eq!(req.cmd, Command::SubscribeStatus);
            vec![
                Response::ok(ResponseData::Status(sample_status())),
                Response::ok(ResponseData::Status(sample_status())),
            ]
        });
        let client = Client::connect(&socket).unwrap();
        let mut stream = client.subscribe().unwrap();
        assert_eq!(stream.next().unwrap().unwrap(), sample_status());
        assert_eq!(stream.next().unwrap().unwrap(), sample_status());
        // Stub closes the connection after serving the one request.
        match stream.next() {
            Some(Err(ClientError::Disconnected)) => {}
            other => panic!("expected disconnect, got {other:?}"),
        }
        assert!(stream.next().is_none());
        server.join().unwrap();
    }
}
