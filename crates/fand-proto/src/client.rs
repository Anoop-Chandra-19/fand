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

use crate::{Command, Request, Response, ResponseData, Status};

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
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "{e}"),
            ClientError::Disconnected => write!(f, "connection closed by daemon"),
            ClientError::Protocol(msg) => write!(f, "{msg}"),
            ClientError::Daemon(msg) => write!(f, "daemon: {msg}"),
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

    /// Current applied config as TOML text.
    pub fn get_config(&mut self) -> Result<String> {
        match self.request(Command::GetConfig)?.data {
            Some(ResponseData::Config { toml }) => Ok(toml),
            other => Err(unexpected_payload(other)),
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
        if !response.ok {
            return Err(ClientError::Daemon(
                response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string()),
            ));
        }
        Ok(response)
    }
}

fn unexpected_payload(data: Option<ResponseData>) -> ClientError {
    ClientError::Protocol(format!("daemon sent unexpected payload: {data:?}"))
}

/// Iterator over pushed status frames. Yields `Err(Disconnected)` exactly
/// once when the daemon goes away, then `None` — so `for frame in stream`
/// with `frame?` surfaces the disconnect instead of spinning on it.
pub struct StatusStream {
    client: Client,
    done: bool,
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
