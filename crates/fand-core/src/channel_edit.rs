//! Surgical edits to a channel's tuning knobs (`curve` binding, `min_pwm`,
//! `smoothing_seconds`) in a config's raw TOML text — same `toml_edit`
//! approach as [`crate::curve_edit`], so every comment and all formatting
//! elsewhere survives untouched.
//!
//! Business rules (e.g. the min_pwm floors) are not re-checked here —
//! `Config::validate` is the single source of truth for those, applied by
//! the caller after this module hands back the edited TOML text.

use thiserror::Error;
use toml_edit::{value, DocumentMut, Item, Table};

#[derive(Debug, Error)]
pub enum ChannelEditError {
    #[error("parsing current config: {0}")]
    Parse(#[from] toml_edit::TomlError),
    #[error("channel `{0}` does not exist")]
    UnknownChannel(String),
    #[error("channel `{0}` is not a table")]
    ChannelNotATable(String),
}

fn channel_table<'a>(
    doc: &'a mut DocumentMut,
    channel: &str,
) -> Result<&'a mut Table, ChannelEditError> {
    doc.get_mut("channels")
        .and_then(Item::as_table_mut)
        .and_then(|channels| channels.get_mut(channel))
        .ok_or_else(|| ChannelEditError::UnknownChannel(channel.to_string()))?
        .as_table_mut()
        .ok_or_else(|| ChannelEditError::ChannelNotATable(channel.to_string()))
}

/// Rebind which curve drives a channel.
pub fn set_channel_curve(
    toml_text: &str,
    channel: &str,
    curve: &str,
) -> Result<String, ChannelEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let chan = channel_table(&mut doc, channel)?;
    chan["curve"] = value(curve);
    Ok(doc.to_string())
}

/// Set `min_pwm` on a channel.
pub fn set_min_pwm(toml_text: &str, channel: &str, min_pwm: u8) -> Result<String, ChannelEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let chan = channel_table(&mut doc, channel)?;
    chan["min_pwm"] = value(i64::from(min_pwm));
    Ok(doc.to_string())
}

/// Set `smoothing_seconds` on a channel.
pub fn set_smoothing_seconds(
    toml_text: &str,
    channel: &str,
    seconds: u64,
) -> Result<String, ChannelEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let chan = channel_table(&mut doc, channel)?;
    chan["smoothing_seconds"] = value(i64::try_from(seconds).unwrap_or(i64::MAX));
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = "\
# pwm1 comment stays
[channels.pwm1]
hwmon_name = \"nct6799\"
curve = \"cpu_rad\"
min_pwm = 80
smoothing_seconds = 12

# pwm2 comment stays
[channels.pwm2]
hwmon_name = \"nct6799\"
curve = \"case_mix\"
min_pwm = 70
smoothing_seconds = 5
";

    #[test]
    fn set_channel_curve_rebinds() {
        let out = set_channel_curve(TOML, "pwm2", "cpu_rad").unwrap();
        assert!(out.contains("# pwm2 comment stays"));
        assert!(!out.contains("case_mix"));
        assert_eq!(out.matches("curve = \"cpu_rad\"").count(), 2);
    }

    #[test]
    fn set_channel_curve_rejects_unknown_channel() {
        assert!(matches!(
            set_channel_curve(TOML, "pwm9", "x"),
            Err(ChannelEditError::UnknownChannel(c)) if c == "pwm9"
        ));
    }

    #[test]
    fn set_min_pwm_keeps_comments() {
        let out = set_min_pwm(TOML, "pwm2", 90).unwrap();
        assert!(out.contains("# pwm1 comment stays"));
        assert!(out.contains("# pwm2 comment stays"));
        assert!(out.contains("min_pwm = 90"));
        assert!(out.contains("min_pwm = 80"), "pwm1 untouched");
    }

    #[test]
    fn set_min_pwm_rejects_unknown_channel() {
        assert!(matches!(
            set_min_pwm(TOML, "pwm9", 90),
            Err(ChannelEditError::UnknownChannel(c)) if c == "pwm9"
        ));
    }

    #[test]
    fn set_smoothing_seconds_updates_value() {
        let out = set_smoothing_seconds(TOML, "pwm1", 20).unwrap();
        assert!(out.contains("smoothing_seconds = 20"));
        assert!(out.contains("smoothing_seconds = 5"), "pwm2 untouched");
    }

}
