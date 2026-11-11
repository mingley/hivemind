//! Applicability: whether a claim holds in the reader's context.
//!
//! For code, validity is often a version range or an environment rather than a
//! calendar interval. A claim carries conditions such as `os = macos` or
//! `dep:cargo/openssl-sys = ">=0.9.100, <0.10"`; the reader sends a context
//! fingerprint, and each read evaluates the conditions against it.

use std::collections::BTreeMap;

use serde::Serialize;

/// The reader's context fingerprint: repository, branch, OS, architecture,
/// toolchain and dependency versions.
pub type Context = BTreeMap<String, String>;

/// Condition keys with these prefixes hold version requirements (semver).
const VERSION_KEY_PREFIXES: &[&str] = &["dep:", "toolchain:", "version:"];

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Applicability {
    /// The claim has no conditions.
    Unconditional,
    /// Every condition matched the context.
    Match,
    /// No condition failed, but the context lacks some keys.
    Unknown { keys: Vec<String> },
    /// A condition failed.
    Mismatch {
        key: String,
        expected: String,
        actual: String,
    },
}

impl Applicability {
    pub fn is_mismatch(&self) -> bool {
        matches!(self, Applicability::Mismatch { .. })
    }
}

pub fn evaluate(conditions: &BTreeMap<String, String>, context: &Context) -> Applicability {
    if conditions.is_empty() {
        return Applicability::Unconditional;
    }
    let mut unknown = Vec::new();
    for (key, expected) in conditions {
        let Some(actual) = context.get(key) else {
            unknown.push(key.clone());
            continue;
        };
        match condition_holds(key, expected, actual) {
            Some(true) => {}
            Some(false) => {
                return Applicability::Mismatch {
                    key: key.clone(),
                    expected: expected.clone(),
                    actual: actual.clone(),
                };
            }
            None => unknown.push(key.clone()),
        }
    }
    if unknown.is_empty() {
        Applicability::Match
    } else {
        Applicability::Unknown { keys: unknown }
    }
}

/// `None` when the condition can't be evaluated, such as an unparseable version.
fn condition_holds(key: &str, expected: &str, actual: &str) -> Option<bool> {
    if VERSION_KEY_PREFIXES.iter().any(|p| key.starts_with(p)) {
        let requirement = semver::VersionReq::parse(expected.trim()).ok()?;
        let version = lenient_version(actual)?;
        return Some(requirement.matches(&version));
    }
    let actual = normalize(key, actual);
    Some(expected.split('|').any(|alternative| {
        let alternative = normalize(key, alternative);
        match alternative.strip_suffix('*') {
            Some(prefix) => actual.starts_with(prefix),
            None => actual == alternative,
        }
    }))
}

/// Accepts `v1.2.3`, `1.2`, and `1` as well as full semver.
fn lenient_version(text: &str) -> Option<semver::Version> {
    let text = text.trim().trim_start_matches('v');
    if let Ok(v) = semver::Version::parse(text) {
        return Some(v);
    }
    let parts = text.split('.').count();
    let padded = match parts {
        1 => format!("{text}.0.0"),
        2 => format!("{text}.0"),
        _ => return None,
    };
    semver::Version::parse(&padded).ok()
}

/// Lowercases, and maps common synonyms for operating systems and architectures.
fn normalize(key: &str, value: &str) -> String {
    let value = value.trim().to_lowercase();
    let canonical = match key {
        "os" => match value.as_str() {
            "darwin" | "osx" | "mac" | "macosx" => "macos",
            "win" | "win32" | "win64" => "windows",
            _ => return value,
        },
        "arch" => match value.as_str() {
            "aarch64" | "armv8" => "arm64",
            "amd64" | "x64" | "x86-64" => "x86_64",
            _ => return value,
        },
        _ => return value,
    };
    canonical.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn no_conditions_is_unconditional() {
        assert_eq!(
            evaluate(&map(&[]), &map(&[("os", "linux")])),
            Applicability::Unconditional
        );
    }

    #[test]
    fn matches_with_synonyms() {
        let conditions = map(&[("os", "macos"), ("arch", "arm64")]);
        let context = map(&[("os", "Darwin"), ("arch", "aarch64")]);
        assert_eq!(evaluate(&conditions, &context), Applicability::Match);
    }

    #[test]
    fn mismatch_wins_over_unknown() {
        let conditions = map(&[("os", "macos"), ("branch", "main")]);
        let context = map(&[("os", "linux")]);
        assert!(evaluate(&conditions, &context).is_mismatch());
    }

    #[test]
    fn missing_keys_are_unknown() {
        let conditions = map(&[("os", "macos"), ("arch", "arm64")]);
        let context = map(&[("os", "macos")]);
        assert_eq!(
            evaluate(&conditions, &context),
            Applicability::Unknown {
                keys: vec!["arch".into()]
            }
        );
    }

    #[test]
    fn dependency_versions_use_semver() {
        let conditions = map(&[("dep:cargo/openssl-sys", ">=0.9.100, <0.10")]);
        let inside = map(&[("dep:cargo/openssl-sys", "0.9.103")]);
        let outside = map(&[("dep:cargo/openssl-sys", "0.10.1")]);
        let short = map(&[("dep:cargo/openssl-sys", "v0.9")]);
        assert_eq!(evaluate(&conditions, &inside), Applicability::Match);
        assert!(evaluate(&conditions, &outside).is_mismatch());
        // 0.9.0 is below the range.
        assert!(evaluate(&conditions, &short).is_mismatch());
    }

    #[test]
    fn unparseable_versions_are_unknown() {
        let conditions = map(&[("dep:npm/react", "^18")]);
        let context = map(&[("dep:npm/react", "workspace:*")]);
        assert!(matches!(
            evaluate(&conditions, &context),
            Applicability::Unknown { .. }
        ));
    }

    #[test]
    fn alternatives_and_prefixes() {
        let conditions = map(&[("os", "macos|linux"), ("branch", "release/*")]);
        assert_eq!(
            evaluate(
                &conditions,
                &map(&[("os", "linux"), ("branch", "release/1.4")])
            ),
            Applicability::Match
        );
        assert!(evaluate(&conditions, &map(&[("os", "linux"), ("branch", "main")])).is_mismatch());
    }
}
