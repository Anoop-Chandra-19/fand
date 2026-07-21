//! mockd — a fake fand daemon for developing the GUI and fanctl against
//! deterministic, scriptable data. Speaks the full v2 socket protocol on a
//! Unix socket; never touches sysfs, NVML, or a real config file (SetConfig
//! applies in memory only and answers `persistence: dry_run`).
//!
//! Temps are synthetic, but PWM targets come from evaluating the *real*
//! curve trees (fand-core) against them, with the channel's offset, floor
//! and ramp applied — so edits made in the GUI change fan behavior on the
//! next tick, exactly like the live daemon.
//!
//! Run through `make dev-mock`, or directly:
//!
//!     cargo run -p fand --example mockd -- --socket /tmp/mockd.sock
//!
//! Scenarios (`--scenario`):
//!   normal      temps drift gently around idle values
//!   heat-ramp   temps sweep ~35→92 °C and back every 3 min — walks every
//!               curve across its whole range
//!   flappy      additionally drops every client every 20 s — exercises the
//!               GUI's daemon-down / reconnect path
//!   restart     like flappy, but comes back as a "new daemon": fresh
//!               instance token, generation reset to 0, overrides gone

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use fand_core::eval::CurveTree;
use fand_core::Config;
use fand_proto::{
    ChannelStatus, Command, ConfigVersion, Persistence, Request, Response, ResponseData,
    SetConfigResult, Status, PROTOCOL_VERSION,
};

/// How often flappy/restart scenarios drop their clients.
const FLAP_EVERY: Duration = Duration::from_secs(20);

/// Same bound as the real engine: a forgotten override must always hand
/// control back to the curves eventually.
const MAX_OVERRIDE_TTL_S: u64 = 3600;

#[derive(Parser)]
#[command(name = "mockd", about = "fake fand daemon with synthetic sensors")]
struct Args {
    /// Unix socket path to serve on
    #[arg(long)]
    socket: PathBuf,

    /// Config file to load (and re-read on reload_config). Never written.
    #[arg(long, default_value = "config/fand.example.toml")]
    config: PathBuf,

    #[arg(long, value_enum, default_value_t = Scenario::Normal)]
    scenario: Scenario,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Scenario {
    Normal,
    HeatRamp,
    Flappy,
    Restart,
}

impl Scenario {
    fn flaps(self) -> bool {
        matches!(self, Scenario::Flappy | Scenario::Restart)
    }
}

/// The mock's whole world, guarded by one lock: what the real daemon
/// spreads across the engine thread lives here.
struct MockState {
    toml: String,
    config: Config,
    version: ConfigVersion,
    overrides: BTreeMap<String, MockOverride>,
}

struct MockOverride {
    pwm: u8,
    expires_at: Instant,
}

struct Shared {
    state: Mutex<MockState>,
    hub: Hub,
    /// Bumped to hang up on every subscriber (flappy/restart scenarios).
    /// A subscriber remembers the epoch it connected under and closes the
    /// connection as soon as the value moves.
    epoch: AtomicU64,
    /// Set by mutating commands so the tick thread publishes a fresh frame
    /// immediately — mirrors the real engine's post-apply retick, so the GUI
    /// sees the new generation without waiting out the tick interval.
    kick: Mutex<bool>,
    kick_cond: Condvar,
}

impl Shared {
    fn kick_tick(&self) {
        *self.kick.lock().unwrap() = true;
        self.kick_cond.notify_all();
    }
}

/// Minimal stand-in for the daemon's StatusHub: latest frame + a condvar so
/// subscribers push as soon as the tick thread publishes.
#[derive(Default)]
struct Hub {
    latest: Mutex<Option<(u64, Status)>>,
    cond: Condvar,
}

impl Hub {
    fn publish(&self, status: Status) {
        let mut latest = self.latest.lock().unwrap();
        let seq = latest.as_ref().map_or(1, |(seq, _)| seq + 1);
        *latest = Some((seq, status));
        self.cond.notify_all();
    }

    fn latest(&self) -> Option<(u64, Status)> {
        self.latest.lock().unwrap().clone()
    }

    fn wait_newer(&self, last_seq: u64, timeout: Duration) -> Option<(u64, Status)> {
        let guard = self.latest.lock().unwrap();
        let (guard, _) = self
            .cond
            .wait_timeout_while(guard, timeout, |latest| {
                latest.as_ref().is_none_or(|(seq, _)| *seq <= last_seq)
            })
            .unwrap();
        guard.clone().filter(|(seq, _)| *seq > last_seq)
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let toml = std::fs::read_to_string(&args.config)
        .with_context(|| format!("reading config {}", args.config.display()))?;
    let config = Config::from_toml_str(&toml)
        .with_context(|| format!("loading config {}", args.config.display()))?;

    let shared = Arc::new(Shared {
        state: Mutex::new(MockState {
            toml,
            config,
            version: ConfigVersion {
                instance: random_instance(),
                generation: 0,
            },
            overrides: BTreeMap::new(),
        }),
        hub: Hub::default(),
        epoch: AtomicU64::new(0),
        kick: Mutex::new(false),
        kick_cond: Condvar::new(),
    });

    if let Some(dir) = args.socket.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let _ = std::fs::remove_file(&args.socket);
    let listener = UnixListener::bind(&args.socket)
        .with_context(|| format!("binding {}", args.socket.display()))?;
    eprintln!(
        "mockd: listening on {} (scenario: {:?})",
        args.socket.display(),
        args.scenario
    );

    {
        let shared = Arc::clone(&shared);
        let scenario = args.scenario;
        thread::spawn(move || tick_loop(&shared, scenario));
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let shared = Arc::clone(&shared);
                let config_path = args.config.clone();
                thread::spawn(move || {
                    let _ = handle_client(stream, &shared, &config_path);
                });
            }
            Err(e) => eprintln!("mockd: accept failed: {e}"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tick thread: synthesize temps, run the real curve trees, publish frames.

fn tick_loop(shared: &Shared, scenario: Scenario) {
    let start = Instant::now();
    let mut trees: BTreeMap<String, CurveTree> = BTreeMap::new();
    // Trees hold per-channel smoothing/hysteresis state, so they are only
    // rebuilt when the config version moves (mirrors the real engine, which
    // rebuilds channels on every config apply).
    let mut built_for: Option<ConfigVersion> = None;
    let mut current_pwm: BTreeMap<String, u8> = BTreeMap::new();
    let mut last_flap = Instant::now();

    loop {
        let tick;
        {
            let mut state = shared.state.lock().unwrap();
            if built_for != Some(state.version) {
                trees = build_trees(&state.config);
                built_for = Some(state.version);
            }
            tick = Duration::from_secs(state.config.daemon.tick_seconds.max(1));

            let t = start.elapsed().as_secs_f64();
            let temps = synth_temps(&state.config, scenario, t);
            let now = Instant::now();
            // Split borrow: expiring/pruning overrides needs &mut while the
            // config is read alongside.
            let MockState {
                config, overrides, ..
            } = &mut *state;
            overrides.retain(|name, o| config.channels.contains_key(name) && o.expires_at > now);

            let mut channels = BTreeMap::new();
            for (idx, (name, ch)) in config.channels.iter().enumerate() {
                let Some(tree) = trees.get_mut(name) else {
                    continue;
                };
                // Validation guarantees every referenced sensor exists and
                // synth_temps covers all of them, so eval cannot miss.
                let curve_out = tree.eval(&temps, now).unwrap_or(255);
                let raw_target =
                    (i32::from(curve_out) + i32::from(ch.offset_pwm)).clamp(0, 255) as u8;

                let (ramp_target, mode, override_remaining_s) = match overrides.get(name) {
                    Some(o) => (
                        o.pwm,
                        "override",
                        Some(o.expires_at.duration_since(now).as_secs().max(1)),
                    ),
                    None => (raw_target, "curve", None),
                };

                let floored = ramp_target.max(ch.min_pwm);
                let pwm = match current_pwm.get(name) {
                    Some(&cur) if floored > cur => cur + (floored - cur).min(ch.max_step_up),
                    Some(&cur) => cur - (cur - floored).min(ch.max_step_down),
                    None => floored,
                };
                current_pwm.insert(name.clone(), pwm);

                let rpm = synth_rpm(pwm, idx, t);
                channels.insert(
                    name.clone(),
                    ChannelStatus {
                        rpm,
                        current_pwm: pwm,
                        target_pwm: raw_target,
                        mode: mode.to_string(),
                        override_remaining_s,
                    },
                );
            }

            shared.hub.publish(Status {
                temps,
                channels,
                config_generation: state.version.generation,
                instance: state.version.instance,
            });
        }

        if scenario.flaps() && last_flap.elapsed() >= FLAP_EVERY {
            last_flap = Instant::now();
            if scenario == Scenario::Restart {
                let mut state = shared.state.lock().unwrap();
                state.version = ConfigVersion {
                    instance: random_instance(),
                    generation: 0,
                };
                state.overrides.clear();
                eprintln!("mockd: simulating daemon restart (new instance)");
            } else {
                eprintln!("mockd: dropping all clients");
            }
            shared.epoch.fetch_add(1, Ordering::SeqCst);
        }

        sleep_tick(shared, tick);
    }
}

/// Sleep one tick, or less if a mutating command kicks for an early frame.
fn sleep_tick(shared: &Shared, tick: Duration) {
    let guard = shared.kick.lock().unwrap();
    let (mut guard, _) = shared
        .kick_cond
        .wait_timeout_while(guard, tick, |kicked| !*kicked)
        .unwrap();
    *guard = false;
}

fn build_trees(config: &Config) -> BTreeMap<String, CurveTree> {
    let tick = config.daemon.tick_seconds.max(1);
    config
        .channels
        .iter()
        .filter_map(|(name, ch)| {
            let window = usize::try_from((ch.smoothing_seconds / tick).max(1)).unwrap_or(1);
            match CurveTree::build(&config.curves, &ch.curve, window) {
                Ok(tree) => Some((name.clone(), tree)),
                // Unreachable on a validated config; skipping the channel
                // beats bringing the whole mock down.
                Err(e) => {
                    eprintln!("mockd: channel {name}: {e}");
                    None
                }
            }
        })
        .collect()
}

/// Deterministic fake temperatures for every configured sensor, staggered
/// per sensor so multi-sensor mixes visibly trade dominance.
fn synth_temps(config: &Config, scenario: Scenario, t: f64) -> BTreeMap<String, f64> {
    config
        .sensors
        .keys()
        .enumerate()
        .map(|(idx, name)| {
            let i = idx as f64;
            let temp = match scenario {
                Scenario::HeatRamp => {
                    // Triangle wave 35..92 °C, 3 min period — covers the
                    // example curves' whole input range in both directions.
                    let phase = (t / 180.0).fract();
                    let tri = if phase < 0.5 {
                        phase * 2.0
                    } else {
                        2.0 - phase * 2.0
                    };
                    35.0 + tri * 57.0 + i * 1.5
                }
                _ => 42.0 + i * 7.0 + 8.0 * (t / 37.0 + i * 1.7).sin() + 1.2 * (t / 5.0 + i).sin(),
            };
            (name.clone(), (temp * 10.0).round() / 10.0)
        })
        .collect()
}

/// Plausible RPM for a PWM value, with a little wobble so it looks alive.
fn synth_rpm(pwm: u8, channel_idx: usize, t: f64) -> u32 {
    let base = 220.0 + f64::from(pwm) * 6.5 + channel_idx as f64 * 90.0;
    let wobble = 25.0 * (t / 7.0 + channel_idx as f64).sin();
    (base + wobble).max(0.0) as u32
}

// ---------------------------------------------------------------------------
// Socket serving: same framing rules as the real server (newline-delimited
// JSON, version probe first, unterminated final records discarded).

fn handle_client(stream: UnixStream, shared: &Shared, config_path: &Path) -> Result<()> {
    let mut writer = stream.try_clone().context("cloning stream")?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(()); // clean EOF
        }
        // Only newline-terminated requests execute (see the real server:
        // this keeps a client's "known not-applied" report on send failure
        // truthful).
        if !line.ends_with('\n') {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
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
            Command::SubscribeStatus => return subscribe(&mut writer, shared),
            cmd => send(&mut writer, &respond(cmd, shared, config_path))?,
        }
    }
}

/// Push the current frame, then every newer one — until the client hangs up
/// or a flap epoch bump tells us to hang up on *them*.
fn subscribe(writer: &mut UnixStream, shared: &Shared) -> Result<()> {
    let epoch = shared.epoch.load(Ordering::SeqCst);
    let mut last_seq = 0;
    if let Some((seq, status)) = shared.hub.latest() {
        last_seq = seq;
        send(writer, &Response::ok(ResponseData::Status(status)))?;
    }
    loop {
        if shared.epoch.load(Ordering::SeqCst) != epoch {
            return Ok(());
        }
        if let Some((seq, status)) = shared.hub.wait_newer(last_seq, Duration::from_secs(1)) {
            last_seq = seq;
            send(writer, &Response::ok(ResponseData::Status(status)))?;
        }
    }
}

fn respond(cmd: Command, shared: &Shared, config_path: &Path) -> Response {
    match cmd {
        Command::GetStatus => match shared.hub.latest() {
            Some((_, status)) => Response::ok(ResponseData::Status(status)),
            None => Response::err("no status yet (first tick pending)"),
        },
        Command::GetConfig => {
            let state = shared.state.lock().unwrap();
            Response::ok(ResponseData::Config {
                toml: state.toml.clone(),
                generation: state.version.generation,
                instance: state.version.instance,
            })
        }
        Command::SetConfig { toml, expected } => {
            let result = set_config(shared, toml, expected);
            Response::ok(ResponseData::SetConfig(result))
        }
        Command::ReloadConfig => reload_config(shared, config_path),
        Command::SetOverride {
            channel,
            pwm,
            ttl_seconds,
        } => set_override(shared, &channel, pwm, ttl_seconds),
        Command::ClearOverride { channel } => clear_override(shared, &channel),
        Command::SubscribeStatus => unreachable!("handled by the connection loop"),
    }
}

/// Compare-and-set config apply, in memory only: the same outcomes as the
/// real engine, but `persistence: dry_run` — mockd never writes files.
fn set_config(shared: &Shared, toml: String, expected: ConfigVersion) -> SetConfigResult {
    let mut state = shared.state.lock().unwrap();
    if expected != state.version {
        return SetConfigResult::Conflict {
            current: state.version,
        };
    }
    match Config::from_toml_str(&toml) {
        Err(e) => SetConfigResult::Rejected {
            error: e.to_string(),
        },
        Ok(config) => {
            state.config = config;
            state.toml = toml.clone();
            state.version.generation += 1;
            let version = state.version;
            drop(state);
            shared.kick_tick();
            SetConfigResult::Applied {
                toml,
                version,
                persistence: Persistence::DryRun,
            }
        }
    }
}

fn reload_config(shared: &Shared, config_path: &Path) -> Response {
    let toml = match std::fs::read_to_string(config_path) {
        Ok(toml) => toml,
        Err(e) => return Response::err(format!("reading {}: {e}", config_path.display())),
    };
    match Config::from_toml_str(&toml) {
        Err(e) => Response::err(e.to_string()),
        Ok(config) => {
            let mut state = shared.state.lock().unwrap();
            state.config = config;
            state.toml = toml;
            state.version.generation += 1;
            drop(state);
            shared.kick_tick();
            Response::ok_empty()
        }
    }
}

fn set_override(shared: &Shared, channel: &str, pwm: u8, ttl_seconds: u64) -> Response {
    let mut state = shared.state.lock().unwrap();
    let Some(ch) = state.config.channels.get(channel) else {
        return unknown_channel(&state, channel);
    };
    let floor = ch.min_pwm;
    if pwm < floor {
        return Response::err(format!(
            "pwm {pwm} is below channel `{channel}`'s floor {floor} — refusing \
             (overrides cannot push a fan into its stall region)"
        ));
    }
    let ttl = ttl_seconds.clamp(1, MAX_OVERRIDE_TTL_S);
    state.overrides.insert(
        channel.to_string(),
        MockOverride {
            pwm,
            expires_at: Instant::now() + Duration::from_secs(ttl),
        },
    );
    drop(state);
    shared.kick_tick();
    Response::ok_empty()
}

fn clear_override(shared: &Shared, channel: &str) -> Response {
    let mut state = shared.state.lock().unwrap();
    if !state.config.channels.contains_key(channel) {
        return unknown_channel(&state, channel);
    }
    state.overrides.remove(channel);
    drop(state);
    shared.kick_tick();
    Response::ok_empty()
}

fn unknown_channel(state: &MockState, channel: &str) -> Response {
    let known: Vec<&str> = state.config.channels.keys().map(String::as_str).collect();
    Response::err(format!(
        "unknown channel `{channel}` (configured: {})",
        known.join(", ")
    ))
}

fn send(writer: &mut UnixStream, response: &Response) -> Result<()> {
    let mut line = serde_json::to_string(response).context("encoding response")?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .context("writing response")
}

/// Random per-process instance token, like the real daemon's — /dev/urandom
/// with a monotonic fallback (never 0, so it can't collide with proto
/// defaults).
fn random_instance() -> u64 {
    let mut buf = [0u8; 8];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut buf).is_ok() {
            let n = u64::from_le_bytes(buf);
            if n != 0 {
                return n;
            }
        }
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64 | 1)
}
