//! Visibility scopes: `global`, `org:<name>`, `team:<name>`,
//! `project:<name>`, `agent:<id>`, `session:<id>`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const MAX_NAME_LEN: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid scope `{0}`; expected `global` or `<org|team|project|agent|session>:<name>` with no whitespace"
)]
pub struct ScopeParseError(pub String);

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    Global,
    Org(String),
    Team(String),
    Project(String),
    Agent(String),
    Session(String),
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_LEN
        && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl Scope {
    /// Checks a scope built directly from its variants, which bypasses
    /// [`FromStr`]. Writes call this so every stored scope parses on replay.
    pub fn validate(&self) -> Result<(), ScopeParseError> {
        match self.name() {
            Some(name) if !valid_name(name) => Err(ScopeParseError(self.to_string())),
            _ => Ok(()),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Org(_) => "org",
            Scope::Team(_) => "team",
            Scope::Project(_) => "project",
            Scope::Agent(_) => "agent",
            Scope::Session(_) => "session",
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Scope::Global => None,
            Scope::Org(n)
            | Scope::Team(n)
            | Scope::Project(n)
            | Scope::Agent(n)
            | Scope::Session(n) => Some(n),
        }
    }

    /// Higher is narrower. Used to rank results from more specific scopes first.
    pub fn specificity(&self) -> u8 {
        match self {
            Scope::Global => 0,
            Scope::Org(_) => 1,
            Scope::Team(_) => 2,
            Scope::Project(_) => 3,
            Scope::Agent(_) => 4,
            Scope::Session(_) => 5,
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            None => f.write_str("global"),
            Some(name) => write!(f, "{}:{name}", self.kind()),
        }
    }
}

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Scope {
    type Err = ScopeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ScopeParseError(s.to_owned());
        if s == "global" {
            return Ok(Scope::Global);
        }
        let (kind, name) = s.split_once(':').ok_or_else(err)?;
        if !valid_name(name) {
            return Err(err());
        }
        let name = name.to_owned();
        match kind {
            "org" => Ok(Scope::Org(name)),
            "team" => Ok(Scope::Team(name)),
            "project" => Ok(Scope::Project(name)),
            "agent" => Ok(Scope::Agent(name)),
            "session" => Ok(Scope::Session(name)),
            _ => Err(err()),
        }
    }
}

impl Serialize for Scope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints() {
        for text in [
            "global",
            "org:acme",
            "team:infra",
            "project:github.com/acme/api",
            "agent:claude-code@laptop-a",
            "session:9f3c",
        ] {
            let scope: Scope = text.parse().unwrap();
            assert_eq!(scope.to_string(), text);
        }
    }

    #[test]
    fn rejects_bad_scopes() {
        for text in [
            "",
            "project",
            "project:",
            "planet:earth",
            "project:a b",
            "Global",
        ] {
            assert!(text.parse::<Scope>().is_err(), "{text}");
        }
    }

    #[test]
    fn validate_catches_scopes_built_directly() {
        assert!(Scope::Project("My Projects/app".into()).validate().is_err());
        assert!(Scope::Team(String::new()).validate().is_err());
        assert!(Scope::Project("acme/api".into()).validate().is_ok());
        assert!(Scope::Global.validate().is_ok());
    }

    #[test]
    fn specificity_orders_narrow_above_broad() {
        let project: Scope = "project:x".parse().unwrap();
        assert!(project.specificity() > Scope::Global.specificity());
    }
}
