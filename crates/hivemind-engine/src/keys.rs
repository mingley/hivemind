//! Write-time key expansion (hippocampal C1): enrich the lexical index with
//! normalized error codes, paths, symbols, and package@version forms without
//! mutating the stored claim.

use std::fmt::Write as _;

/// Extra index text derived from a claim's statement and entity keys.
pub fn expand(statement: &str, about: &[String]) -> String {
    let mut out = String::new();
    for entity in about {
        push_entity(&mut out, entity);
    }
    for token in statement.split_whitespace() {
        let t = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && !"_./:@+-#".contains(c));
        if t.is_empty() {
            continue;
        }
        if looks_like_path(t) {
            let _ = write!(out, "\npath:{t}");
            if let Some(file) = t.rsplit('/').next() {
                let _ = write!(out, "\nfile:{file}");
            }
        }
        if looks_like_error_code(t) {
            let _ = write!(out, "\nerr:{}", t.to_ascii_uppercase());
        }
        if t.contains("::") {
            let _ = write!(out, "\nsym:{t}");
        }
        if let Some((name, ver)) = split_pkg_version(t) {
            let _ = write!(out, "\npkg:{name}@{ver}");
        }
    }
    out
}

fn push_entity(out: &mut String, entity: &str) {
    let _ = write!(out, "\n{entity}");
    if let Some(rest) = entity.strip_prefix("pkg:") {
        let _ = write!(out, "\npkg:{rest}");
        if let Some((eco, name)) = rest.split_once('/') {
            let _ = write!(out, "\n{eco}:{name}");
        }
    } else if let Some(rest) = entity.strip_prefix("err:") {
        let _ = write!(out, "\nerr:{}", rest.to_ascii_uppercase());
    }
}

fn looks_like_path(t: &str) -> bool {
    t.contains('/')
        && t.chars().any(|c| c.is_ascii_alphanumeric())
        && !t.starts_with("http:")
        && !t.starts_with("https:")
}

fn looks_like_error_code(t: &str) -> bool {
    let upper = t.to_ascii_uppercase();
    (upper.starts_with('E') && upper.chars().skip(1).all(|c| c.is_ascii_digit()) && t.len() >= 3)
        || upper.starts_with("ERR_")
        || (upper.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            && upper.contains('_')
            && upper.len() >= 6)
}

fn split_pkg_version(t: &str) -> Option<(&str, &str)> {
    let (name, ver) = t.split_once('@')?;
    if name.is_empty() || ver.is_empty() {
        return None;
    }
    if ver.chars().next()?.is_ascii_digit() {
        Some((name, ver))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_paths_errors_and_packages() {
        let text = expand(
            "E401 on src/auth/mod.rs: openssl-sys@0.9.103 ::SslContext",
            &["pkg:cargo/openssl-sys".into(), "err:e401".into()],
        );
        assert!(text.contains("path:src/auth/mod.rs"));
        assert!(text.contains("file:mod.rs"));
        assert!(text.contains("err:E401"));
        assert!(text.contains("pkg:openssl-sys@0.9.103"));
        assert!(text.contains("sym:::SslContext") || text.contains("SslContext"));
        assert!(text.contains("pkg:cargo/openssl-sys"));
    }
}
