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

/// Wire form: `{"version":1,"cmd":"get_status"}`. Config and override
/// commands land in phase 5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    GetStatus,
    SubscribeStatus,
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
}

/// One snapshot of the daemon's world, produced once per control tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// °C by sensor name — the raw reads from the latest tick (smoothing
    /// happens per channel, not per sensor).
    pub temps: BTreeMap<String, f64>,
    pub channels: BTreeMap<String, ChannelStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelStatus {
    pub rpm: u32,
    /// PWM actually written this tick (ramp output).
    pub current_pwm: u8,
    /// Raw curve/mix target this tick, before hysteresis/ramping.
    pub target_pwm: u8,
    /// "manual" for now; overrides and failsafe states arrive later.
    pub mode: String,
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
                    mode: "manual".into(),
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
    fn requests_round_trip() {
        for cmd in [Command::GetStatus, Command::SubscribeStatus] {
            let req = Request::new(cmd);
            let json = serde_json::to_string(&req).unwrap();
            assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);
        }
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
