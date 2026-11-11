//! Core types for hivemind: identifiers, time, scopes, claims, events,
//! applicability, belief labels, and write hygiene.
//!
//! This crate is pure data and logic with no I/O, so every transport (CLI, MCP,
//! gRPC) and the storage engine share one definition of the memory model
//! (`docs/03-memory-model.md`).

/// Error returned when a string does not name a variant of a closed enum.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} `{input}`; expected one of: {}", expected.join(", "))]
pub struct ParseEnumError {
    pub kind: &'static str,
    pub input: String,
    pub expected: &'static [&'static str],
}

/// Defines a closed enum with a stable snake_case wire name. Parsing also
/// accepts kebab-case, so CLI users can type `not-applicable`.
macro_rules! str_enum {
    ($(#[$meta:meta])* pub enum $name:ident { $($(#[$vmeta:meta])* $variant:ident => $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $wire)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $wire),+
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::ParseEnumError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let normalized = s.trim().to_ascii_lowercase().replace('-', "_");
                match normalized.as_str() {
                    $($wire => Ok($name::$variant),)+
                    _ => Err($crate::ParseEnumError {
                        kind: stringify!($name),
                        input: s.to_owned(),
                        expected: &[$($wire),+],
                    }),
                }
            }
        }
    };
}

pub mod applicability;
pub mod belief;
pub mod claim;
pub mod event;
pub mod hygiene;
pub mod ids;
pub mod scope;
pub mod time;

pub use applicability::{Applicability, Context};
pub use belief::{BELIEF_POLICY, Status, TrustTier, Verification};
pub use claim::{
    Citation, ClaimData, ClaimKind, EvidenceRef, Method, Origin, Provenance, ValidationError,
};
pub use event::{
    Actor, ActorKind, EVENT_VERSION, EdgeKind, Event, EventBody, Outcome, RetractReason,
};
pub use ids::{BlobHash, ClaimId, EventId, IdParseError};
pub use scope::{Scope, ScopeParseError};
pub use time::{TimeParseError, Timestamp};
