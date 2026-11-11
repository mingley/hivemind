//! Typed identifiers.
//!
//! Claims and events use UUIDv7: globally unique and time-ordered, so logs
//! written by different stores can later be merged as a set union. Artifacts
//! are addressed by the BLAKE3 hash of their content.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} id `{input}`")]
pub struct IdParseError {
    pub kind: &'static str,
    pub input: String,
}

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// A new identifier, ordered by creation time.
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}_{}", $prefix, self.0.simple())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let raw = s.strip_prefix(concat!($prefix, "_")).unwrap_or(s);
                Uuid::try_parse(raw).map(Self).map_err(|_| IdParseError {
                    kind: $prefix,
                    input: s.to_owned(),
                })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

uuid_id!(
    /// Identifies one claim (memory item) across its whole lifecycle.
    ClaimId,
    "clm"
);
uuid_id!(
    /// Identifies one event in the log.
    EventId,
    "evt"
);

/// BLAKE3 hash of an artifact's bytes; the artifact's address in the blob store.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobHash([u8; 32]);

impl BlobHash {
    const PREFIX: &'static str = "blake3:";

    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex, 64 characters.
    pub fn to_hex(&self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", Self::PREFIX, self.to_hex())
    }
}

impl fmt::Debug for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for BlobHash {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = s.strip_prefix(Self::PREFIX).unwrap_or(s);
        blake3::Hash::from_hex(raw)
            .map(|h| Self(*h.as_bytes()))
            .map_err(|_| IdParseError {
                kind: "blake3",
                input: s.to_owned(),
            })
    }
}

impl Serialize for BlobHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for BlobHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_id_round_trips_with_and_without_prefix() {
        let id = ClaimId::generate();
        let text = id.to_string();
        assert!(text.starts_with("clm_"));
        assert_eq!(text.parse::<ClaimId>().unwrap(), id);
        assert_eq!(text["clm_".len()..].parse::<ClaimId>().unwrap(), id);
        assert!("evt_nothex".parse::<ClaimId>().is_err());
    }

    #[test]
    fn ids_are_time_ordered() {
        let a = ClaimId::generate();
        let b = ClaimId::generate();
        assert!(a < b);
    }

    #[test]
    fn blob_hash_round_trips() {
        let h = BlobHash::of(b"hello");
        assert_eq!(h.to_string().parse::<BlobHash>().unwrap(), h);
        assert_eq!(h.to_hex().parse::<BlobHash>().unwrap(), h);
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<BlobHash>(&json).unwrap(), h);
    }
}
