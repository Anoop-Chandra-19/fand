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
    /// Pin a channel to a fixed PWM for a while (the daemon enforces the
    /// channel's safety floor; expires automatically)
    Override {
        /// Channel name, e.g. pwm2
        channel: String,
        /// PWM 0-255, or a duty percentage like 55%
        #[arg(required_unless_present = "clear")]
        value: Option<String>,
        /// Seconds until the override expires (daemon caps at 3600)
        #[arg(long, default_value_t = 60)]
        ttl: u64,
        /// Clear the active override instead of setting one
        #[arg(long, conflicts_with_all = ["value", "ttl"])]
        clear: bool,
    },
    /// Inspect or change the daemon's config
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Inspect or edit fan curves
    #[command(subcommand)]
    Curve(CurveCmd),
}

#[derive(Subcommand)]
enum CurveCmd {
    /// Print curve points (all curves, or one by name)
    Show { name: Option<String> },
    /// Replace a curve's points, e.g.: fanctl curve set cpu 40:80 60:55% 80:100%
    Set {
        name: String,
        /// temp:pwm pairs; pwm as raw 0-255 or a percentage
        #[arg(required = true)]
        points: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the currently applied config (TOML)
    Show,
    /// Make the daemon re-read /etc/fand/config.toml
    Reload,
    /// Edit the config in $EDITOR; validated locally, then hot-applied
    /// and persisted by the daemon
    Edit,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let result = match args.cmd {
        Cli::Status => status(&args.socket),
        Cli::Watch => watch(&args.socket),
        Cli::Override {
            channel,
            value,
            ttl,
            clear,
        } => override_cmd(&args.socket, &channel, value.as_deref(), ttl, clear),
        Cli::Config(ConfigCmd::Show) => config_show(&args.socket),
        Cli::Config(ConfigCmd::Reload) => config_reload(&args.socket),
        Cli::Config(ConfigCmd::Edit) => config_edit(&args.socket),
        Cli::Curve(CurveCmd::Show { name }) => curve_show(&args.socket, name.as_deref()),
        Cli::Curve(CurveCmd::Set { name, points }) => curve_set(&args.socket, &name, &points),
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

fn override_cmd(
    socket: &Path,
    channel: &str,
    value: Option<&str>,
    ttl: u64,
    clear: bool,
) -> Result<()> {
    let (request, done_msg) = if clear {
        (
            Request::new(Command::ClearOverride {
                channel: channel.to_string(),
            }),
            format!("{channel}: override cleared — back to curve"),
        )
    } else {
        let pwm = parse_pwm(value.expect("clap requires a value unless --clear"))?;
        (
            Request::new(Command::SetOverride {
                channel: channel.to_string(),
                pwm,
                ttl_seconds: ttl,
            }),
            format!(
                "{channel}: pinned at pwm {pwm} ({}) for {ttl}s",
                duty_percent(pwm)
            ),
        )
    };
    let mut stream = connect(socket)?;
    send(&mut stream, &request)?;
    expect_ok(&mut BufReader::new(stream))?;
    println!("{done_msg}");
    Ok(())
}

fn config_show(socket: &Path) -> Result<()> {
    print!("{}", fetch_config(socket)?);
    Ok(())
}

fn config_reload(socket: &Path) -> Result<()> {
    let mut stream = connect(socket)?;
    send(&mut stream, &Request::new(Command::ReloadConfig))?;
    expect_ok(&mut BufReader::new(stream))?;
    println!("config reloaded and applied");
    Ok(())
}

/// $EDITOR round-trip: fetch → edit → validate locally (with a re-edit
/// loop, so a typo never costs you the whole edit) → send. The daemon
/// re-validates and only persists after the config applied to hardware.
fn config_edit(socket: &Path) -> Result<()> {
    let original = fetch_config(socket)?;
    let dir = tempfile::tempdir().context("creating temp dir")?;
    let path = dir.path().join("fand-config.toml");
    std::fs::write(&path, &original).context("writing temp config")?;

    loop {
        edit_in_editor(&path)?;
        let edited = std::fs::read_to_string(&path).context("reading edited config")?;
        if edited == original {
            println!("no changes — nothing to apply");
            return Ok(());
        }
        match fand_core::Config::from_toml_str(&edited) {
            Ok(_) => {
                let mut stream = connect(socket)?;
                send(&mut stream, &Request::new(Command::SetConfig { toml: edited }))?;
                expect_ok(&mut BufReader::new(stream))?;
                println!("config applied and persisted");
                return Ok(());
            }
            Err(e) => {
                eprintln!("invalid config:\n  {e}");
                if !ask_yes_no("re-edit?")? {
                    bail!("aborted — daemon config unchanged");
                }
            }
        }
    }
}

fn edit_in_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let status = std::process::Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("launching editor `{editor}`"))?;
    if !status.success() {
        bail!("editor `{editor}` exited with {status}");
    }
    Ok(())
}

fn ask_yes_no(prompt: &str) -> Result<bool> {
    use std::io::Write as _;
    print!("{prompt} [Y/n] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(!answer.trim().eq_ignore_ascii_case("n"))
}

fn curve_show(socket: &Path, name: Option<&str>) -> Result<()> {
    let cfg = fand_core::Config::from_toml_str(&fetch_config(socket)?)
        .map_err(|e| anyhow!("daemon sent a config that does not validate: {e}"))?;
    let selected: Vec<(&String, &fand_core::config::CurveConfig)> = match name {
        Some(n) => vec![(
            cfg.curves
                .get_key_value(n)
                .with_context(|| {
                    let known: Vec<&str> = cfg.curves.keys().map(String::as_str).collect();
                    format!("unknown curve `{n}` (configured: {})", known.join(", "))
                })?
                .0,
            &cfg.curves[n],
        )],
        None => cfg.curves.iter().collect(),
    };
    for (i, (curve_name, curve)) in selected.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("curve `{curve_name}`:");
        println!("  {:>5}{:>7}{:>6}", "°C", "DUTY", "PWM");
        for &(temp, pwm) in &curve.points {
            println!("  {:>5}{:>7}{:>6}", temp, duty_percent(pwm as u8), pwm);
        }
    }
    Ok(())
}

fn curve_set(socket: &Path, name: &str, point_args: &[String]) -> Result<()> {
    let points = point_args
        .iter()
        .map(|s| parse_point(s))
        .collect::<Result<Vec<_>>>()?;
    let current = fetch_config(socket)?;
    let updated = replace_curve_points(&current, name, &points)?;
    // Instant local feedback; the daemon re-validates anyway.
    fand_core::Config::from_toml_str(&updated)
        .map_err(|e| anyhow!("resulting config would be invalid: {e}"))?;
    let mut stream = connect(socket)?;
    send(&mut stream, &Request::new(Command::SetConfig { toml: updated }))?;
    expect_ok(&mut BufReader::new(stream))?;
    println!(
        "curve `{name}` set to {} point(s), applied and persisted",
        points.len()
    );
    Ok(())
}

/// `40:80` or `40:31%` → (temp °C, raw pwm).
fn parse_point(s: &str) -> Result<(i32, u8)> {
    let (temp, pwm) = s
        .split_once(':')
        .with_context(|| format!("bad point `{s}` (want temp:pwm, e.g. 40:80 or 40:31%)"))?;
    let temp: i32 = temp
        .parse()
        .with_context(|| format!("bad temperature in `{s}`"))?;
    Ok((temp, parse_pwm(pwm)?))
}

/// Surgically replace `[curves.<name>].points` in the TOML text, keeping
/// every comment and all formatting elsewhere intact (why this uses
/// toml_edit rather than a parse/re-serialize round trip).
fn replace_curve_points(toml_text: &str, name: &str, points: &[(i32, u8)]) -> Result<String> {
    let mut doc: toml_edit::DocumentMut =
        toml_text.parse().context("parsing current config")?;
    let curves = doc
        .entry("curves")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("[curves] is not a table")?;
    let is_new = !curves.contains_key(name);
    let curve = curves
        .entry(name)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .with_context(|| format!("[curves.{name}] is not a table"))?;

    let mut arr = toml_edit::Array::new();
    for &(temp, pwm) in points {
        let mut pair = toml_edit::Array::new();
        pair.push(i64::from(temp));
        pair.push(i64::from(pwm));
        arr.push(toml_edit::Value::Array(pair));
    }
    curve["points"] = toml_edit::value(arr);
    if is_new {
        eprintln!("note: curve `{name}` did not exist — creating it (assign it to a channel to take effect)");
    }
    Ok(doc.to_string())
}

fn fetch_config(socket: &Path) -> Result<String> {
    let mut stream = connect(socket)?;
    send(&mut stream, &Request::new(Command::GetConfig))?;
    match read_response(&mut BufReader::new(stream))?.data {
        Some(ResponseData::Config { toml }) => Ok(toml),
        other => bail!("daemon sent unexpected payload: {other:?}"),
    }
}

/// Accept a raw PWM (`140`) or a duty percentage (`55%`) — the wire always
/// carries raw 0-255.
fn parse_pwm(s: &str) -> Result<u8> {
    if let Some(pct) = s.strip_suffix('%') {
        let pct: f64 = pct
            .trim()
            .parse()
            .with_context(|| format!("bad percentage `{s}`"))?;
        if !(0.0..=100.0).contains(&pct) {
            bail!("percentage {pct} out of range 0-100");
        }
        Ok((pct * 255.0 / 100.0).round() as u8)
    } else {
        let raw: u16 = s
            .parse()
            .with_context(|| format!("bad pwm value `{s}` (want 0-255 or a percentage like 55%)"))?;
        if raw > 255 {
            bail!("pwm {raw} out of range 0-255");
        }
        Ok(raw as u8)
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

/// Read one response line; a daemon-side error becomes our error.
fn read_response(reader: &mut BufReader<UnixStream>) -> Result<Response> {
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
    Ok(response)
}

/// For commands whose success carries no payload.
fn expect_ok(reader: &mut BufReader<UnixStream>) -> Result<()> {
    read_response(reader).map(|_| ())
}

fn read_status(reader: &mut BufReader<UnixStream>) -> Result<Status> {
    match read_response(reader)?.data {
        Some(ResponseData::Status(status)) => Ok(status),
        Some(other) => bail!("daemon sent unexpected payload: {other:?}"),
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
        "{:<10}{:>6}{:>7}{:>8}  MODE",
        "CHANNEL", "RPM", "DUTY", "TARGET"
    );
    for (name, ch) in &status.channels {
        let mode = match ch.override_remaining_s {
            Some(s) => format!("override ({s}s left)"),
            None => ch.mode.clone(),
        };
        println!(
            "{:<10}{:>6}{:>7}{:>8}  {}",
            name,
            ch.rpm,
            duty_percent(ch.current_pwm),
            duty_percent(ch.target_pwm),
            mode
        );
    }
}

/// Raw PWM (0-255) as a duty percentage — what the wire carries vs what a
/// human wants to read.
fn duty_percent(pwm: u8) -> String {
    format!("{}%", (f64::from(pwm) * 100.0 / 255.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::{duty_percent, parse_point, parse_pwm};

    #[test]
    fn duty_percent_endpoints_and_rounding() {
        assert_eq!(duty_percent(0), "0%");
        assert_eq!(duty_percent(255), "100%");
        assert_eq!(duty_percent(128), "50%");
        // The pump-safety floor on pwm1.
        assert_eq!(duty_percent(80), "31%");
    }

    #[test]
    fn parse_pwm_raw_values() {
        assert_eq!(parse_pwm("0").unwrap(), 0);
        assert_eq!(parse_pwm("140").unwrap(), 140);
        assert_eq!(parse_pwm("255").unwrap(), 255);
        assert!(parse_pwm("256").is_err());
        assert!(parse_pwm("-1").is_err());
        assert!(parse_pwm("fast").is_err());
    }

    #[test]
    fn parse_point_pairs() {
        assert_eq!(parse_point("40:80").unwrap(), (40, 80));
        assert_eq!(parse_point("60:55%").unwrap(), (60, 140));
        assert_eq!(parse_point("-5:100").unwrap(), (-5, 100));
        assert!(parse_point("40").is_err());
        assert!(parse_point("hot:80").is_err());
        assert!(parse_point("40:300").is_err());
    }

    #[test]
    fn replace_curve_points_keeps_comments() {
        let toml = "\
# top comment stays
[curves.cpu]
# curve comment stays
points = [[40, 80], [70, 200]]

[curves.gpu]
points = [[35, 70], [80, 255]] # trailing comment stays
";
        let out = super::replace_curve_points(toml, "cpu", &[(30, 70), (60, 140), (85, 255)])
            .unwrap();
        assert!(out.contains("# top comment stays"));
        assert!(out.contains("# curve comment stays"));
        assert!(out.contains("# trailing comment stays"));
        assert!(out.contains("points = [[30, 70], [60, 140], [85, 255]]"));
        assert!(out.contains("[[35, 70], [80, 255]]"), "other curve untouched");
    }

    #[test]
    fn replace_curve_points_can_create_a_curve() {
        let toml = "[curves.cpu]\npoints = [[40, 80], [70, 200]]\n";
        let out = super::replace_curve_points(toml, "case", &[(30, 70), (80, 255)]).unwrap();
        assert!(out.contains("[curves.case]"));
        assert!(out.contains("points = [[30, 70], [80, 255]]"));
    }

    #[test]
    fn parse_pwm_percentages() {
        assert_eq!(parse_pwm("0%").unwrap(), 0);
        assert_eq!(parse_pwm("100%").unwrap(), 255);
        assert_eq!(parse_pwm("55%").unwrap(), 140);
        assert_eq!(parse_pwm("31%").unwrap(), 79);
        assert!(parse_pwm("101%").is_err());
        assert!(parse_pwm("-5%").is_err());
    }
}
