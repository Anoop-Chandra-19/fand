//! Surgical edits to a channel's `Policy` in a config's raw TOML text —
//! reassign which curve drives a sensor input, or add/remove a `mix`
//! input — using the same `toml_edit` approach as [`crate::curve_edit`] so
//! every comment and all formatting elsewhere survives untouched.
//!
//! Every channel keeps its policy *shape*: this module never converts a
//! `single` channel to `mix` or back, and never touches which sensors
//! exist — only which curve(s) a channel's existing input(s) reference.

use thiserror::Error;
use toml_edit::{value, DocumentMut, InlineTable, Item, Table, Value};

#[derive(Debug, Error)]
pub enum PolicyEditError {
    #[error("parsing current config: {0}")]
    Parse(#[from] toml_edit::TomlError),
    #[error("channel `{0}` does not exist")]
    UnknownChannel(String),
    #[error("channel `{0}` is not a table")]
    ChannelNotATable(String),
    #[error("channel `{channel}` has no input for sensor `{sensor}`")]
    UnknownInput { channel: String, sensor: String },
    #[error("channel `{channel}` is not a `mix` policy")]
    NotMixPolicy { channel: String },
    #[error("channel `{channel}` already has an input for sensor `{sensor}`")]
    DuplicateSensor { channel: String, sensor: String },
    #[error("channel `{0}` needs at least one mix input")]
    CannotRemoveLastInput(String),
}

fn channel_table<'a>(
    doc: &'a mut DocumentMut,
    channel: &str,
) -> Result<&'a mut Table, PolicyEditError> {
    doc.get_mut("channels")
        .and_then(Item::as_table_mut)
        .and_then(|channels| channels.get_mut(channel))
        .ok_or_else(|| PolicyEditError::UnknownChannel(channel.to_string()))?
        .as_table_mut()
        .ok_or_else(|| PolicyEditError::ChannelNotATable(channel.to_string()))
}

fn sensor_of(v: &Value) -> Option<&str> {
    v.as_inline_table()?.get("sensor")?.as_str()
}

/// Reassign the curve for one sensor input on a channel: the single
/// `curve` key for a `single` policy, or the matching entry in `inputs`
/// for a `mix` policy.
pub fn set_channel_curve(
    toml_text: &str,
    channel: &str,
    sensor: &str,
    curve: &str,
) -> Result<String, PolicyEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let chan = channel_table(&mut doc, channel)?;

    if chan.contains_key("curve") {
        chan["curve"] = value(curve);
    } else {
        let inputs = chan
            .get_mut("inputs")
            .and_then(Item::as_array_mut)
            .ok_or_else(|| PolicyEditError::UnknownInput {
                channel: channel.to_string(),
                sensor: sensor.to_string(),
            })?;
        let entry = inputs
            .iter_mut()
            .find(|v| sensor_of(v) == Some(sensor))
            .and_then(Value::as_inline_table_mut)
            .ok_or_else(|| PolicyEditError::UnknownInput {
                channel: channel.to_string(),
                sensor: sensor.to_string(),
            })?;
        entry.insert("curve", Value::from(curve.to_string()));
    }
    Ok(doc.to_string())
}

/// Append a new `{ sensor, curve }` input to a `mix` channel.
pub fn add_mix_input(
    toml_text: &str,
    channel: &str,
    sensor: &str,
    curve: &str,
) -> Result<String, PolicyEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let chan = channel_table(&mut doc, channel)?;
    let inputs = chan
        .get_mut("inputs")
        .and_then(Item::as_array_mut)
        .ok_or_else(|| PolicyEditError::NotMixPolicy {
            channel: channel.to_string(),
        })?;

    if inputs.iter().any(|v| sensor_of(v) == Some(sensor)) {
        return Err(PolicyEditError::DuplicateSensor {
            channel: channel.to_string(),
            sensor: sensor.to_string(),
        });
    }

    let mut entry = InlineTable::new();
    entry.insert("sensor", Value::from(sensor.to_string()));
    entry.insert("curve", Value::from(curve.to_string()));
    inputs.push(Value::InlineTable(entry));
    Ok(doc.to_string())
}

/// Remove the input for `sensor` from a `mix` channel. Refuses to drop
/// the last remaining input — a mix with zero inputs is meaningless, and
/// the daemon rejects it anyway (`EmptyMix`).
pub fn remove_mix_input(
    toml_text: &str,
    channel: &str,
    sensor: &str,
) -> Result<String, PolicyEditError> {
    let mut doc: DocumentMut = toml_text.parse()?;
    let chan = channel_table(&mut doc, channel)?;
    let inputs = chan
        .get_mut("inputs")
        .and_then(Item::as_array_mut)
        .ok_or_else(|| PolicyEditError::NotMixPolicy {
            channel: channel.to_string(),
        })?;

    if inputs.len() <= 1 {
        return Err(PolicyEditError::CannotRemoveLastInput(channel.to_string()));
    }

    let index = inputs.iter().position(|v| sensor_of(v) == Some(sensor));
    match index {
        Some(i) => {
            inputs.remove(i);
            Ok(doc.to_string())
        }
        None => Err(PolicyEditError::UnknownInput {
            channel: channel.to_string(),
            sensor: sensor.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = "\
# pwm1 comment stays
[channels.pwm1]
hwmon_name = \"nct6799\"
policy = \"single\"
sensor = \"cpu\"
curve = \"cpu_rad\"
min_pwm = 80
smoothing_seconds = 12

# pwm2 comment stays
[channels.pwm2]
hwmon_name = \"nct6799\"
policy = \"mix\"
inputs = [{ sensor = \"cpu\", curve = \"cpu_case\" }, { sensor = \"gpu\", curve = \"gpu_case\" }]
min_pwm = 70
smoothing_seconds = 5
";

    #[test]
    fn set_channel_curve_updates_single_policy() {
        let out = set_channel_curve(TOML, "pwm1", "cpu", "new_curve").unwrap();
        assert!(out.contains("# pwm1 comment stays"));
        assert!(out.contains("curve = \"new_curve\""));
        assert!(out.contains("curve = \"cpu_case\""), "pwm2 untouched");
    }

    #[test]
    fn set_channel_curve_updates_one_mix_input() {
        let out = set_channel_curve(TOML, "pwm2", "gpu", "new_gpu_curve").unwrap();
        assert!(out.contains("# pwm2 comment stays"));
        assert!(out.contains("sensor = \"cpu\", curve = \"cpu_case\""), "cpu input untouched");
        assert!(out.contains("sensor = \"gpu\", curve = \"new_gpu_curve\""));
        assert!(out.contains("curve = \"cpu_rad\""), "pwm1 untouched");
    }

    #[test]
    fn set_channel_curve_rejects_unknown_channel() {
        assert!(matches!(
            set_channel_curve(TOML, "pwm9", "cpu", "x"),
            Err(PolicyEditError::UnknownChannel(c)) if c == "pwm9"
        ));
    }

    #[test]
    fn set_channel_curve_rejects_unknown_sensor_in_mix() {
        assert!(matches!(
            set_channel_curve(TOML, "pwm2", "nope", "x"),
            Err(PolicyEditError::UnknownInput { sensor, .. }) if sensor == "nope"
        ));
    }

    #[test]
    fn add_mix_input_appends_entry() {
        let out = add_mix_input(TOML, "pwm2", "ssd", "case_extra").unwrap();
        assert!(out.contains("sensor = \"ssd\""));
        assert!(out.contains("curve = \"case_extra\""));
        assert!(out.contains("sensor = \"cpu\", curve = \"cpu_case\""), "existing inputs untouched");
    }

    #[test]
    fn add_mix_input_rejects_duplicate_sensor() {
        assert!(matches!(
            add_mix_input(TOML, "pwm2", "cpu", "x"),
            Err(PolicyEditError::DuplicateSensor { sensor, .. }) if sensor == "cpu"
        ));
    }

    #[test]
    fn add_mix_input_rejects_non_mix_channel() {
        assert!(matches!(
            add_mix_input(TOML, "pwm1", "cpu", "x"),
            Err(PolicyEditError::NotMixPolicy { channel }) if channel == "pwm1"
        ));
    }

    #[test]
    fn remove_mix_input_removes_entry() {
        let out = remove_mix_input(TOML, "pwm2", "gpu").unwrap();
        assert!(!out.contains("gpu_case"));
        assert!(out.contains("sensor = \"cpu\", curve = \"cpu_case\""));
    }

    #[test]
    fn remove_mix_input_rejects_last_input() {
        let single_input_toml = "\
[channels.pwm2]
hwmon_name = \"nct6799\"
policy = \"mix\"
inputs = [{ sensor = \"cpu\", curve = \"cpu_case\" }]
min_pwm = 70
smoothing_seconds = 5
";
        assert!(matches!(
            remove_mix_input(single_input_toml, "pwm2", "cpu"),
            Err(PolicyEditError::CannotRemoveLastInput(c)) if c == "pwm2"
        ));
    }
}
