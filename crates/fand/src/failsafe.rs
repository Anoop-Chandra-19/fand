//! Restoring firmware-auto fan control (`pwmN_enable = 5`) on every exit
//! path: a Drop guard for clean exits and unwinding panics, a panic hook
//! for panic=abort and panics on other threads, and `restore_all` for the
//! `--restore-auto` subcommand (systemd ExecStopPost, covers SIGKILL).

use std::fs;
use std::path::PathBuf;

/// pwmN_enable value for firmware auto on the NCT6799 (verified on the
/// target board; `1` would be manual).
pub const FIRMWARE_AUTO: &str = "5";

/// Best-effort restore: logs failures instead of returning them, because
/// this runs inside Drop and the panic hook where erroring out is not an
/// option — every remaining channel must still be attempted.
pub fn restore_all(enable_paths: &[PathBuf]) {
    for path in enable_paths {
        match fs::write(path, FIRMWARE_AUTO) {
            Ok(()) => eprintln!("fand: restored firmware auto: {}", path.display()),
            Err(e) => eprintln!(
                "fand: FAILED to restore firmware auto on {}: {e}",
                path.display()
            ),
        }
    }
}

/// Construct BEFORE taking manual control of any channel; keep alive for
/// the daemon's whole lifetime. Drop hands every channel back to firmware.
pub struct FailsafeGuard {
    enable_paths: Vec<PathBuf>,
}

impl FailsafeGuard {
    pub fn new(enable_paths: Vec<PathBuf>) -> Self {
        Self { enable_paths }
    }
}

impl Drop for FailsafeGuard {
    fn drop(&mut self) {
        eprintln!("fand: restoring firmware-auto fan control");
        restore_all(&self.enable_paths);
    }
}

/// Chain a restore onto the default panic handler. The Drop guard already
/// covers unwinding panics on the main thread; the hook additionally covers
/// panic=abort builds and panics on other threads.
pub fn install_panic_hook(enable_paths: Vec<PathBuf>) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        eprintln!("fand: panic — restoring firmware-auto fan control");
        restore_all(&enable_paths);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn guard_drop_writes_firmware_auto() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("pwm1_enable");
        let p2 = dir.path().join("pwm2_enable");
        fs::write(&p1, "1").unwrap();
        fs::write(&p2, "1").unwrap();

        drop(FailsafeGuard::new(vec![p1.clone(), p2.clone()]));

        assert_eq!(fs::read_to_string(&p1).unwrap(), "5");
        assert_eq!(fs::read_to_string(&p2).unwrap(), "5");
    }

    #[test]
    fn restore_all_continues_past_failures() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope/pwm1_enable");
        let good = dir.path().join("pwm2_enable");
        fs::write(&good, "1").unwrap();

        // The unwritable path must not stop the good one from being restored.
        restore_all(&[missing, good.clone()]);
        assert_eq!(fs::read_to_string(&good).unwrap(), "5");
    }
}
