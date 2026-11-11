//! Line-delimited JSON protocol between clients and `hivemindd`.
//!
//! See `docs/m1-implementation-notes.md` for the method list and framing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL: &str = "hivemind-socket-v1";
pub const RUN_DIR: &str = "run";
pub const SOCK_NAME: &str = "hivemindd.sock";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: String,
    pub version: String,
    pub store_id: String,
    pub next_offset: u64,
}

impl Response {
    pub fn ok(id: u64, result: impl Serialize) -> Self {
        Self {
            id,
            ok: true,
            result: Some(serde_json::to_value(result).expect("response value")),
            error: None,
        }
    }

    pub fn err(id: u64, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

/// Socket path: `<store>/run/hivemindd.sock`.
pub fn socket_path(store_dir: &std::path::Path) -> std::path::PathBuf {
    store_dir.join(RUN_DIR).join(SOCK_NAME)
}
