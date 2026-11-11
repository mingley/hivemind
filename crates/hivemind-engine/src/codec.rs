//! Event encoding. The first byte names the codec, so the format can evolve
//! without rewriting history; readers keep every codec they ever wrote.

use hivemind_core::Event;

use crate::error::{Error, Result};

/// serde_json encoding of [`Event`]. Readable with standard tools, and struct
/// fields serialize in declaration order, so encodings are deterministic.
pub const JSON_V1: u8 = 1;

pub fn encode(event: &Event) -> Result<Vec<u8>> {
    let mut bytes = vec![JSON_V1];
    serde_json::to_writer(&mut bytes, event).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(bytes)
}

pub fn decode(offset: u64, bytes: &[u8]) -> Result<Event> {
    match bytes.split_first() {
        Some((&JSON_V1, json)) => serde_json::from_slice(json).map_err(|e| Error::Decode {
            offset,
            reason: e.to_string(),
        }),
        Some((codec, _)) => Err(Error::Decode {
            offset,
            reason: format!("unknown codec {codec}"),
        }),
        None => Err(Error::Decode {
            offset,
            reason: "empty event".into(),
        }),
    }
}
