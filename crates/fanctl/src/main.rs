//! fanctl — unprivileged CLI client for the fand daemon (user must be in
//! the `fand` group). Speaks fand-proto over the daemon's Unix socket.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use fand_core::config::CurveConfig;
use fand_proto::client::{Client, ClientError};
use fand_proto::{Command, Persistence, SetConfigResult, Status, SOCKET_PATH};

#[derive(Parser)]
#[command(
    name = "fanctl",
    version,
    about = "inspect and control the fand daemon"
)]
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
    /// Print curves (all, or one by name)
    Show { name: Option<String> },
    /// Replace a graph curve's points, e.g.: fanctl curve set cpu 40:80 60:55% 80:100%
    Set {
        name: String,
        /// temp:pwm pairs; pwm as raw 0-255 or a percentage
        #[arg(required = true)]
        points: Vec<String>,
        /// Create the curve bound to this sensor if it doesn't exist yet
        #[arg(long)]
        sensor: Option<String>,
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
        Cli::Curve(CurveCmd::Set {
            name,
            points,
            sensor,
        }) => curve_set(&args.socket, &name, &points, sensor.as_deref()),
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
    let status = connect(socket)?.get_status()?;
    print_status(&status);
    Ok(())
}

fn watch(socket: &Path) -> Result<()> {
    for frame in connect(socket)?.subscribe()? {
        let status = frame?;
        // Clear screen + move cursor home, then redraw.
        print!("\x1b[2J\x1b[H");
        print_status(&status);
        println!("\nlive — Ctrl-C to quit");
    }
    Ok(())
}

fn override_cmd(
    socket: &Path,
    channel: &str,
    value: Option<&str>,
    ttl: u64,
    clear: bool,
) -> Result<()> {
    let (cmd, done_msg) = if clear {
        (
            Command::ClearOverride {
                channel: channel.to_string(),
            },
            format!("{channel}: override cleared — back to curve"),
        )
    } else {
        let pwm = parse_pwm(value.expect("clap requires a value unless --clear"))?;
        (
            Command::SetOverride {
                channel: channel.to_string(),
                pwm,
                ttl_seconds: ttl,
            },
            format!(
                "{channel}: pinned at pwm {pwm} ({}) for {ttl}s",
                duty_percent(pwm)
            ),
        )
    };
    connect(socket)?.request_mutating(cmd)?;
    println!("{done_msg}");
    Ok(())
}

fn config_show(socket: &Path) -> Result<()> {
    print!("{}", fetch_config(socket)?);
    Ok(())
}

fn config_reload(socket: &Path) -> Result<()> {
    connect(socket)?.request_mutating(Command::ReloadConfig)?;
    println!("config reloaded and applied");
    Ok(())
}

/// $EDITOR round-trip: fetch → edit → validate locally (with a re-edit
/// loop, so a typo never costs you the whole edit) → send with the
/// fetched version as the CAS `expected`. The editor can hold the
/// snapshot for minutes; if anything else (the GUI, another fanctl)
/// changed the config meanwhile, the daemon reports a conflict instead
/// of silently overwriting that change — and the edit is preserved to a
/// durable path, never discarded.
fn config_edit(socket: &Path) -> Result<()> {
    let snap = connect(socket)?.get_config()?;
    let dir = tempfile::tempdir().context("creating temp dir")?;
    let path = dir.path().join("fand-config.toml");
    std::fs::write(&path, &snap.toml).context("writing temp config")?;

    loop {
        edit_in_editor(&path)?;
        let edited = std::fs::read_to_string(&path).context("reading edited config")?;
        if edited == snap.toml {
            println!("no changes — nothing to apply");
            return Ok(());
        }
        if let Err(e) = fand_core::Config::from_toml_str(&edited) {
            eprintln!("invalid config:\n  {e}");
            if !ask_yes_no("re-edit?")? {
                bail!("aborted — daemon config unchanged");
            }
            continue;
        }
        match connect(socket)?.set_config(edited.clone(), snap.version) {
            Ok(SetConfigResult::Applied { persistence, .. }) => {
                print_applied(persistence);
                return Ok(());
            }
            Ok(SetConfigResult::AppliedButNotPersisted { error, .. }) => {
                println!("config applied");
                eprintln!("warning: {error}");
                return Ok(());
            }
            Ok(SetConfigResult::Conflict { current }) => {
                let kept = keep_edit(dir, &edited);
                bail!(
                    "the daemon's config changed while you were editing (your edit was \
                     based on {}; the daemon is now at {current}) — nothing was applied.\n\
                     your edited version is kept at {kept}\n\
                     re-run `fanctl config edit` to redo the change against the current config",
                    snap.version,
                );
            }
            Ok(SetConfigResult::Rejected { error }) => {
                eprintln!("daemon rejected the config:\n  {error}");
                if !ask_yes_no("re-edit?")? {
                    bail!("aborted — daemon config unchanged");
                }
            }
            Err(ClientError::OutcomeUnknown(cause)) => {
                let kept = keep_edit(dir, &edited);
                bail!(
                    "the daemon did not confirm the change ({cause}) — it may or may \
                     not have applied. check `fanctl config show`; your edited version \
                     is kept at {kept}"
                );
            }
            Err(e) => {
                let kept = keep_edit(dir, &edited);
                return Err(
                    anyhow!(e).context(format!("sending the edited config (kept at {kept})"))
                );
            }
        }
    }
}

fn print_applied(persistence: Persistence) {
    match persistence {
        Persistence::Persisted => println!("config applied and persisted"),
        Persistence::DryRun => {
            println!("config applied (dry-run daemon: in memory only, not persisted)")
        }
    }
}

/// Where a not-applied edit survives, as a printable location: preferably
/// copied (crash-durably) to the state dir; if even that fails, the temp
/// dir itself is kept (its auto-delete disarmed) — the "never discarded"
/// guarantee has no failure mode that drops the only copy. No durability
/// is claimed for the fallback: the platform temp dir may be tmpfs (gone
/// on reboot) or disk that tmpfiles cleanup can empty without one, so the
/// message promises nothing and tells the user to move the file.
fn keep_edit(dir: tempfile::TempDir, edited: &str) -> String {
    match preserve_edit(edited) {
        Ok(path) => path.display().to_string(),
        Err(e) => {
            let dir = dir.keep();
            format!(
                "{} — a TEMP directory not guaranteed to survive cleanup or a reboot, \
                 copy the file somewhere safe now (saving to the state dir failed: {e:#})",
                dir.join("fand-config.toml").display()
            )
        }
    }
}

/// Copy a not-applied edit out of the auto-deleted temp dir so user work
/// is never lost. Lands in XDG state (`~/.local/state/fanctl/`).
fn preserve_edit(edited: &str) -> Result<PathBuf> {
    let base = state_base(
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
    .context("no absolute XDG_STATE_HOME or HOME — nowhere durable to save the edit")?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    preserve_edit_in(&base.join("fanctl"), stamp, edited)
}

/// The XDG state base directory, or None when no absolute candidate exists.
/// The XDG spec declares a relative value in these variables invalid and to
/// be ignored — honoring that also keeps `preserve_edit_in`'s durability
/// walk on absolute paths only, where the ancestor chain is well-defined
/// all the way up. No temp-dir fallback here: a "rescue" copied into
/// temporary storage would be presented as durable while carrying the same
/// lifecycle risk the keep-the-tempdir path warns loudly about — better to
/// fail into that honest fallback than to succeed into a quiet one.
fn state_base(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    xdg.filter(|p| p.is_absolute()).or_else(|| {
        home.filter(|p| p.is_absolute())
            .map(|h| h.join(".local/state"))
    })
}

fn preserve_edit_in(dir: &Path, stamp: u64, edited: &str) -> Result<PathBuf> {
    // Everything below this ancestor is about to be created and needs its
    // directory entry synced; everything at or above it already existed,
    // and syncing it would only add failure modes (some filesystems reject
    // directory fsync) that could reject an already-durable rescue file.
    let sync_stop = dir
        .ancestors()
        .find(|a| a.exists())
        .unwrap_or_else(|| Path::new("/"));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    // create_new + a widening suffix: a second conflict in the same
    // second must never overwrite the first preserved edit.
    for attempt in 0u32.. {
        let name = match attempt {
            0 => format!("config-edit-{stamp}.toml"),
            n => format!("config-edit-{stamp}-{n}.toml"),
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                return match write_durably(file, &path, dir, sync_stop, edited) {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        // A half-written file must not linger looking like
                        // a preserved edit. Best-effort — the temp source
                        // still holds the real copy either way — but if
                        // cleanup itself fails, the error must say a
                        // partial file may remain, or the leftover would
                        // pass for a preserved edit later.
                        let cleanup = std::fs::remove_file(&path)
                            .and_then(|()| std::fs::File::open(dir).and_then(|d| d.sync_all()));
                        Err(match cleanup {
                            Ok(()) => e,
                            Err(c) => e.context(format!(
                                "a partial copy may remain at {} (cleanup failed: {c})",
                                path.display()
                            )),
                        })
                    }
                };
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("creating {}", path.display()));
            }
        }
    }
    unreachable!("create_new loop always returns")
}

/// The temp source is deleted the moment the caller returns Ok, so the copy
/// must actually be on disk first: sync the file's bytes, then the directory
/// entries — the filename is an entry in `dir`, and each directory
/// `create_dir_all` just made is itself an entry in *its* parent. The walk
/// stops at `sync_stop`, the deepest directory that predates this call.
fn write_durably(
    mut file: std::fs::File,
    path: &Path,
    dir: &Path,
    sync_stop: &Path,
    edited: &str,
) -> Result<()> {
    use std::io::Write as _;
    file.write_all(edited.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", path.display()))?;
    for ancestor in dir.ancestors() {
        std::fs::File::open(ancestor)
            .and_then(|d| d.sync_all())
            .with_context(|| format!("syncing {}", ancestor.display()))?;
        if ancestor == sync_stop {
            break;
        }
    }
    Ok(())
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
        match curve {
            CurveConfig::Graph(g) => {
                println!("curve `{curve_name}` (graph, sensor `{}`):", g.sensor);
                println!("  {:>5}{:>7}{:>6}", "°C", "DUTY", "PWM");
                for &(temp, pwm) in &g.points {
                    println!("  {:>5}{:>7}{:>6}", temp, duty_percent(pwm as u8), pwm);
                }
            }
            CurveConfig::Mix(m) => {
                println!(
                    "curve `{curve_name}` (mix, {} of: {})",
                    m.function.as_str(),
                    m.curves.join(", ")
                );
            }
            CurveConfig::Flat(f) => {
                println!(
                    "curve `{curve_name}` (flat: pwm {} = {})",
                    f.pwm,
                    duty_percent(f.pwm as u8)
                );
            }
            CurveConfig::Trigger(t) => {
                println!("curve `{curve_name}` (trigger, sensor `{}`):", t.sensor);
                println!(
                    "  idle ≤ {}°C → pwm {} ({})",
                    t.idle_temp,
                    t.idle_pwm,
                    duty_percent(t.idle_pwm as u8)
                );
                println!(
                    "  load ≥ {}°C → pwm {} ({})",
                    t.load_temp,
                    t.load_pwm,
                    duty_percent(t.load_pwm as u8)
                );
                if t.response_seconds > 0 {
                    println!("  response: {}s", t.response_seconds);
                } else {
                    println!("  response: instant");
                }
            }
        }
    }
    Ok(())
}

fn curve_set(socket: &Path, name: &str, point_args: &[String], sensor: Option<&str>) -> Result<()> {
    let points = point_args
        .iter()
        .map(|s| parse_point(s))
        .collect::<Result<Vec<_>>>()?;
    // One connection for the whole read-modify-write: the fetched version
    // is the CAS `expected`, so a concurrent change (GUI, another fanctl)
    // conflicts instead of being silently overwritten.
    let mut client = connect(socket)?;
    let snap = client.get_config()?;
    let is_new = !fand_core::Config::from_toml_str(&snap.toml)
        .map_err(|e| anyhow!("daemon sent a config that does not validate: {e}"))?
        .curves
        .contains_key(name);
    let updated = match (is_new, sensor) {
        (false, _) => fand_core::replace_curve_points(&snap.toml, name, &points)
            .context("editing curve points")?,
        (true, Some(sensor)) => fand_core::create_graph_curve(&snap.toml, name, sensor, &points)
            .context("creating curve")?,
        (true, None) => bail!(
            "curve `{name}` does not exist — pass --sensor <name> to create it \
             (a graph curve needs a temperature source)"
        ),
    };
    // Instant local feedback; the daemon re-validates anyway.
    fand_core::Config::from_toml_str(&updated)
        .map_err(|e| anyhow!("resulting config would be invalid: {e}"))?;
    match client.set_config(updated, snap.version) {
        Ok(SetConfigResult::Applied { persistence, .. }) => {
            if is_new {
                eprintln!("note: created curve `{name}` (bind it to a channel to take effect)");
            }
            println!("curve `{name}` set to {} point(s)", points.len());
            print_applied(persistence);
            Ok(())
        }
        Ok(SetConfigResult::AppliedButNotPersisted { error, .. }) => {
            println!("curve `{name}` set to {} point(s), applied", points.len());
            eprintln!("warning: {error}");
            Ok(())
        }
        Ok(SetConfigResult::Conflict { current }) => bail!(
            "the daemon's config changed while this command ran (now at {current}) — \
             nothing was changed; re-run"
        ),
        Ok(SetConfigResult::Rejected { error }) => bail!("daemon rejected the config: {error}"),
        Err(ClientError::OutcomeUnknown(cause)) => bail!(
            "the daemon did not confirm the change ({cause}) — it may or may not \
             have applied; check `fanctl curve show {name}`"
        ),
        Err(e) => Err(e.into()),
    }
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

fn fetch_config(socket: &Path) -> Result<String> {
    Ok(connect(socket)?.get_config()?.toml)
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
        let raw: u16 = s.parse().with_context(|| {
            format!("bad pwm value `{s}` (want 0-255 or a percentage like 55%)")
        })?;
        if raw > 255 {
            bail!("pwm {raw} out of range 0-255");
        }
        Ok(raw as u8)
    }
}

fn connect(socket: &Path) -> Result<Client> {
    Client::connect(socket).map_err(|e| {
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
    use super::{duty_percent, parse_point, parse_pwm, preserve_edit_in, state_base};
    use std::path::PathBuf;

    /// The XDG spec says a relative XDG_STATE_HOME is invalid and must be
    /// ignored — fall through to ~/.local/state, not a half-supported
    /// relative path whose durability walk can't reach the cwd entry.
    #[test]
    fn state_base_ignores_relative_xdg_state_home() {
        assert_eq!(
            state_base(
                Some(PathBuf::from("relative/state")),
                Some(PathBuf::from("/home/x"))
            ),
            Some(PathBuf::from("/home/x/.local/state"))
        );
        assert_eq!(
            state_base(
                Some(PathBuf::from("/abs/state")),
                Some(PathBuf::from("/home/x"))
            ),
            Some(PathBuf::from("/abs/state"))
        );
        assert_eq!(
            state_base(None, Some(PathBuf::from("/home/x"))),
            Some(PathBuf::from("/home/x/.local/state"))
        );
        // No absolute candidate at all: None, so preservation fails into
        // the keep-the-tempdir fallback and its honest warning — never a
        // quiet copy into more temporary storage presented as durable.
        assert_eq!(state_base(None, None), None);
    }

    /// A conflicted edit must land, byte-for-byte, at the path we tell
    /// the user about — their work is never discarded.
    #[test]
    fn preserve_edit_writes_content_to_reported_path() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("state").join("fanctl");
        let path = preserve_edit_in(&base, 1_752_800_000, "[daemon]\ntick_seconds = 2\n").unwrap();
        assert_eq!(path, base.join("config-edit-1752800000.toml"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[daemon]\ntick_seconds = 2\n"
        );
    }

    /// Two conflicts in the same second must preserve both edits — the
    /// second gets a suffixed name instead of overwriting the first.
    #[test]
    fn preserve_edit_never_overwrites_an_earlier_preserved_edit() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("fanctl");
        let first = preserve_edit_in(&base, 42, "first edit\n").unwrap();
        let second = preserve_edit_in(&base, 42, "second edit\n").unwrap();
        assert_ne!(first, second);
        assert_eq!(second, base.join("config-edit-42-1.toml"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "first edit\n");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "second edit\n");
    }

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
    fn parse_pwm_percentages() {
        assert_eq!(parse_pwm("0%").unwrap(), 0);
        assert_eq!(parse_pwm("100%").unwrap(), 255);
        assert_eq!(parse_pwm("55%").unwrap(), 140);
        assert_eq!(parse_pwm("31%").unwrap(), 79);
        assert!(parse_pwm("101%").is_err());
        assert!(parse_pwm("-5%").is_err());
    }
}
