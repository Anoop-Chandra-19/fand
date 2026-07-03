//! fanctl — unprivileged CLI client for the fand daemon (user must be in
//! the `fand` group). Speaks fand-proto over the daemon's Unix socket.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use fand_proto::{Command, Request, Response, ResponseData, Status, SOCKET_PATH};

#[derive(Parser)]
#[command(name = "fanctl", version, about = "inspect and control the fand daemon")]
struct Args {
    /// Daemon socket path
    #[arg(long, default_value = SOCKET_PATH)]
    socket: PathBuf,

    #[command(subcommand)]
    cmd: Cli,
}

#[derive(Subcommand)]
enum Cli {
    /// One-shot table of temps, fan RPMs and PWMs
    Status,
    /// Live-updating status view (Ctrl-C to quit)
    Watch,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let result = match args.cmd {
        Cli::Status => status(&args.socket),
        Cli::Watch => watch(&args.socket),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fanctl: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn status(socket: &Path) -> Result<()> {
    let mut stream = connect(socket)?;
    send(&mut stream, &Request::new(Command::GetStatus))?;
    let mut reader = BufReader::new(stream);
    let status = read_status(&mut reader)?;
    print_status(&status);
    Ok(())
}

fn watch(socket: &Path) -> Result<()> {
    let mut stream = connect(socket)?;
    send(&mut stream, &Request::new(Command::SubscribeStatus))?;
    let mut reader = BufReader::new(stream);
    loop {
        let status = read_status(&mut reader)?;
        // Clear screen + move cursor home, then redraw.
        print!("\x1b[2J\x1b[H");
        print_status(&status);
        println!("\nlive — Ctrl-C to quit");
    }
}

fn connect(socket: &Path) -> Result<UnixStream> {
    UnixStream::connect(socket).map_err(|e| {
        let hint = match e.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
                "\nis fand running? (systemctl status fand)"
            }
            ErrorKind::PermissionDenied => {
                "\nare you in the `fand` group? (sudo usermod -aG fand $USER, then re-login)"
            }
            _ => "",
        };
        anyhow!("connecting to {}: {e}{hint}", socket.display())
    })
}

fn send(stream: &mut UnixStream, request: &Request) -> Result<()> {
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stream.write_all(line.as_bytes()).context("sending request")
}

/// Read one response line and unwrap it down to a Status.
fn read_status(reader: &mut BufReader<UnixStream>) -> Result<Status> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).context("reading response")?;
    if n == 0 {
        bail!("connection closed by daemon");
    }
    let response: Response =
        serde_json::from_str(&line).with_context(|| format!("bad response: {}", line.trim()))?;
    if !response.ok {
        bail!(
            "daemon: {}",
            response.error.unwrap_or_else(|| "unknown error".to_string())
        );
    }
    match response.data {
        Some(ResponseData::Status(status)) => Ok(status),
        None => bail!("daemon sent ok response without data"),
    }
}

fn print_status(status: &Status) {
    println!("{:<10}{:>8}", "SENSOR", "°C");
    for (name, temp) in &status.temps {
        println!("{name:<10}{temp:>8.1}");
    }
    println!();
    println!(
        "{:<10}{:>6}{:>6}{:>8}  MODE",
        "CHANNEL", "RPM", "PWM", "TARGET"
    );
    for (name, ch) in &status.channels {
        println!(
            "{:<10}{:>6}{:>6}{:>8}  {}",
            name, ch.rpm, ch.current_pwm, ch.target_pwm, ch.mode
        );
    }
}
