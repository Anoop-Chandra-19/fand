//! fand-proto — socket protocol types shared by daemon, fanctl, and the
//! Tauri backend.
//!
//! Transport: Unix socket /run/fand/fand.sock (root:fand, 0660),
//! newline-delimited JSON. Every message carries a version field.
//!
//! Requests: get_status, subscribe_status (server pushes at 1–2 Hz),
//! get_config / set_config, set_override { channel, pwm, ttl_seconds },
//! clear_override { channel }.
//!
//! Every response: { ok: bool, error?: string, data?: ... }

/// Protocol version stamped into every message.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default socket path.
pub const SOCKET_PATH: &str = "/run/fand/fand.sock";
