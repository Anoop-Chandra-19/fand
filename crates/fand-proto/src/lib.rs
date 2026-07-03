//! fand-proto — socket protocol types shared by daemon, fanctl, and the
//! Tauri backend.
//!
//! Transport: Unix socket /run/fand/fand.sock (root:fand, 0660),
//! newline-delimited JSON. Every message carries a version field.
//!
//! `subscribe_status` turns the connection into a one-way push stream (one
//! status response per daemon tick); everything else is request/response.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub mod client;

/// Protocol version stamped into every message.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default socket path.
pub const SOCKET_PATH: &str = "/run/fand/fand.sock";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    #[serde(flatten)]
    pub cmd: Command,
}

impl Request {
    pub fn new(cmd: Command) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            cmd,
        }
    }
}

/// Wire form: `{"version":1,"cmd":"get_status"}`; variants with fields
/// flatten them alongside the tag, e.g.
/// `{"version":1,"cmd":"set_override","channel":"pwm2","pwm":140,"ttl_seconds":60}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    GetStatus,
    SubscribeStatus,
    /// Current applied config as TOML text (comments preserved).
    GetConfig,
    /// Validate, hot-apply, then persist to the daemon's config path.
    SetConfig { toml: String },
    /// Re-read the config file from disk and hot-apply it.
    ReloadConfig,
    /// Pin a channel to a fixed PWM until the TTL expires. The daemon
    /// enforces the channel's safety floor; this is not a raw write.
    SetOverride {
        channel: String,
        pwm: u8,
        ttl_seconds: u64,
    },
    ClearOverride { channel: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ResponseData>,
}

impl Response {
    pub fn ok(data: ResponseData) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            ok: true,
            error: None,
            data: Some(data),
        }
    }

    /// Success with no payload — for commands that only *do* something
    /// (override, reload) rather than fetch something.
    pub fn ok_empty() -> Self {
        Self {
            version: PROTOCOL_VERSION,
            ok: true,
            error: None,
            data: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            ok: false,
            error: Some(message.into()),
            data: None,
        }
    }
}

/// Tagged so new payload kinds (config, ...) can be added without breaking
/// old clients' parsers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseData {
    Status(Status),
    Config { toml: String },
}

/// One snapshot of the daemon's world, produced once per control tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// °C by sensor name — the raw reads from the latest tick (smoothing
    /// happens per channel, not per sensor).
    pub temps: BTreeMap<String, f64>,
    pub channels: BTreeMap<String, ChannelStatus>,
}

/// PWM values are raw 0-255 (what hwmon and the config speak). Clients
/// display them as duty percentages: round(pwm * 100 / 255).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelStatus {
    pub rpm: u32,
    /// PWM actually written this tick (ramp output).
    pub current_pwm: u8,
    /// Raw curve/mix target this tick, before hysteresis/ramping. Reports
    /// the curve value even while an override is active, so clients can
    /// show what the channel would do on its own.
    pub target_pwm: u8,
    /// "curve" (following its curve/mix policy) or "override" (pinned).
    pub mode: String,
    /// Seconds until an active override expires; absent in curve mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_remaining_s: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_status() -> Status {
        Status {
            temps: BTreeMap::from([("cpu".into(), 54.5)]),
            channels: BTreeMap::from([(
                "pwm2".into(),
                ChannelStatus {
                    rpm: 750,
                    current_pwm: 94,
                    target_pwm: 96,
                    mode: "curve".into(),
                    override_remaining_s: None,
                },
            )]),
        }
    }

    #[test]
    fn request_wire_format_is_stable() {
        let json = serde_json::to_string(&Request::new(Command::GetStatus)).unwrap();
        assert_eq!(json, r#"{"version":1,"cmd":"get_status"}"#);
    }

    #[test]
    fn override_wire_format_is_stable() {
        let cmd = Command::SetOverride {
            channel: "pwm2".into(),
            pwm: 140,
            ttl_seconds: 60,
        };
        let json = serde_json::to_string(&Request::new(cmd)).unwrap();
        assert_eq!(
            json,
            r#"{"version":1,"cmd":"set_override","channel":"pwm2","pwm":140,"ttl_seconds":60}"#
        );
    }

    #[test]
    fn requests_round_trip() {
        let commands = [
            Command::GetStatus,
            Command::SubscribeStatus,
            Command::GetConfig,
            Command::SetConfig {
                toml: "[daemon]\ntick_seconds = 2\n".into(),
            },
            Command::ReloadConfig,
            Command::SetOverride {
                channel: "pwm2".into(),
                pwm: 140,
                ttl_seconds: 60,
            },
            Command::ClearOverride {
                channel: "pwm2".into(),
            },
        ];
        for cmd in commands {
            let req = Request::new(cmd);
            let json = serde_json::to_string(&req).unwrap();
            assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);
        }
    }

    #[test]
    fn config_response_round_trips() {
        let resp = Response::ok(ResponseData::Config {
            toml: "[daemon]\n".into(),
        });
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""kind":"config""#));
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), resp);
    }

    #[test]
    fn status_without_override_omits_remaining_seconds() {
        let json = serde_json::to_string(&sample_status()).unwrap();
        assert!(!json.contains("override_remaining_s"));
    }

    #[test]
    fn unknown_command_is_rejected() {
        assert!(serde_json::from_str::<Request>(r#"{"version":1,"cmd":"reboot"}"#).is_err());
    }

    #[test]
    fn responses_round_trip() {
        let resp = Response::ok(ResponseData::Status(sample_status()));
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), resp);
    }

    #[test]
    fn error_response_omits_data() {
        let json = serde_json::to_string(&Response::err("nope")).unwrap();
        assert_eq!(json, r#"{"version":1,"ok":false,"error":"nope"}"#);
    }
}
