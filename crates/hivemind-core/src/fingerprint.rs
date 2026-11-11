//! Build a reader context fingerprint from lockfiles and the environment.
//!
//! Parses Cargo.lock, package-lock.json, Pipfile.lock, and poetry.lock into
//! `dep:cargo/…`, `dep:npm/…`, and `dep:pip/…` keys for applicability checks.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::applicability::Context;

/// Merge lockfile-derived dependency versions into `ctx`. Existing keys win.
pub fn merge_lockfile_fingerprint(dir: &Path, ctx: &mut Context) {
    for (key, value) in fingerprint_lockfiles(dir) {
        ctx.entry(key).or_insert(value);
    }
}

/// Scan `dir` for supported lockfiles and return dependency keys.
pub fn fingerprint_lockfiles(dir: &Path) -> Context {
    let mut out = Context::new();
    if let Ok(text) = fs::read_to_string(dir.join("Cargo.lock")) {
        merge_map(&mut out, parse_cargo_lock(&text));
    }
    if let Ok(text) = fs::read_to_string(dir.join("package-lock.json")) {
        merge_map(&mut out, parse_npm_lock(&text));
    }
    if let Ok(text) = fs::read_to_string(dir.join("Pipfile.lock")) {
        merge_map(&mut out, parse_pipfile_lock(&text));
    }
    if let Ok(text) = fs::read_to_string(dir.join("poetry.lock")) {
        merge_map(&mut out, parse_poetry_lock(&text));
    }
    out
}

fn merge_map(into: &mut Context, from: Context) {
    for (k, v) in from {
        into.entry(k).or_insert(v);
    }
}

/// `[[package]]` blocks with `name` / `version` lines.
pub fn parse_cargo_lock(text: &str) -> Context {
    let mut out = Context::new();
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            flush_dep(&mut out, "cargo", name.take(), version.take());
            in_package = true;
            continue;
        }
        if !in_package {
            continue;
        }
        if line.starts_with('[') {
            flush_dep(&mut out, "cargo", name.take(), version.take());
            in_package = false;
            continue;
        }
        if let Some(v) = strip_toml_string(line, "name") {
            name = Some(v);
        } else if let Some(v) = strip_toml_string(line, "version") {
            version = Some(v);
        }
    }
    flush_dep(&mut out, "cargo", name, version);
    out
}

/// npm package-lock.json v1 (`dependencies`) and v2/v3 (`packages`).
pub fn parse_npm_lock(text: &str) -> Context {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Context::new();
    };
    let mut out = Context::new();
    if let Some(packages) = value.get("packages").and_then(|v| v.as_object()) {
        for (path, meta) in packages {
            if path.is_empty() {
                continue;
            }
            let name = npm_package_name(path);
            if let Some(version) = meta.get("version").and_then(|v| v.as_str()) {
                out.insert(format!("dep:npm/{name}"), version.to_owned());
            }
        }
        return out;
    }
    if let Some(deps) = value.get("dependencies").and_then(|v| v.as_object()) {
        collect_npm_deps(deps, &mut out);
    }
    out
}

fn collect_npm_deps(deps: &serde_json::Map<String, serde_json::Value>, out: &mut Context) {
    for (name, meta) in deps {
        if let Some(version) = meta.get("version").and_then(|v| v.as_str()) {
            out.insert(format!("dep:npm/{name}"), version.to_owned());
        }
        if let Some(nested) = meta.get("dependencies").and_then(|v| v.as_object()) {
            collect_npm_deps(nested, out);
        }
    }
}

fn npm_package_name(path: &str) -> String {
    let trimmed = path.trim_start_matches("node_modules/");
    // Keep scoped packages: @scope/name may appear after nested node_modules/.
    if let Some(idx) = trimmed.rfind("node_modules/") {
        trimmed[idx + "node_modules/".len()..].to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Pipfile.lock: `default` and `develop` maps with `"version": "==1.2.3"`.
pub fn parse_pipfile_lock(text: &str) -> Context {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Context::new();
    };
    let mut out = Context::new();
    for section in ["default", "develop"] {
        let Some(map) = value.get(section).and_then(|v| v.as_object()) else {
            continue;
        };
        for (name, meta) in map {
            if let Some(version) = meta.get("version").and_then(|v| v.as_str()) {
                let version = version.trim().trim_start_matches('=').trim().to_owned();
                if !version.is_empty() {
                    out.insert(format!("dep:pip/{name}"), version);
                }
            }
        }
    }
    out
}

/// poetry.lock `[[package]]` name/version pairs (TOML subset).
pub fn parse_poetry_lock(text: &str) -> Context {
    let mut out = Context::new();
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            flush_dep(&mut out, "pip", name.take(), version.take());
            in_package = true;
            continue;
        }
        if !in_package {
            continue;
        }
        if line.starts_with('[') {
            flush_dep(&mut out, "pip", name.take(), version.take());
            in_package = line == "[[package]]";
            continue;
        }
        if let Some(v) = strip_toml_string(line, "name") {
            name = Some(v);
        } else if let Some(v) = strip_toml_string(line, "version") {
            version = Some(v);
        }
    }
    flush_dep(&mut out, "pip", name, version);
    out
}

fn strip_toml_string(line: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} =");
    let rest = line.strip_prefix(&prefix)?.trim();
    let rest = rest.strip_prefix('"')?.strip_suffix('"')?;
    Some(rest.to_owned())
}

fn flush_dep(out: &mut Context, ecosystem: &str, name: Option<String>, version: Option<String>) {
    if let (Some(name), Some(version)) = (name, version) {
        out.insert(format!("dep:{ecosystem}/{name}"), version);
    }
}

/// Convenience: os/arch plus lockfiles from `dir`.
pub fn fingerprint_here(dir: &Path) -> Context {
    let mut ctx = BTreeMap::new();
    ctx.insert("os".into(), std::env::consts::OS.to_owned());
    ctx.insert("arch".into(), std::env::consts::ARCH.to_owned());
    merge_lockfile_fingerprint(dir, &mut ctx);
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cargo_lock() {
        let text = r#"
version = 4

[[package]]
name = "serde"
version = "1.0.229"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tokio"
version = "1.49.0"
"#;
        let ctx = parse_cargo_lock(text);
        assert_eq!(
            ctx.get("dep:cargo/serde").map(String::as_str),
            Some("1.0.229")
        );
        assert_eq!(
            ctx.get("dep:cargo/tokio").map(String::as_str),
            Some("1.49.0")
        );
    }

    #[test]
    fn parses_npm_and_pip_locks() {
        let npm = r#"{
          "packages": {
            "": { "name": "app" },
            "node_modules/lodash": { "version": "4.17.21" },
            "node_modules/@scope/pkg": { "version": "1.2.3" }
          }
        }"#;
        let ctx = parse_npm_lock(npm);
        assert_eq!(ctx["dep:npm/lodash"], "4.17.21");
        assert_eq!(ctx["dep:npm/@scope/pkg"], "1.2.3");

        let pip = r#"{
          "default": { "requests": { "version": "==2.32.3" } },
          "develop": { "pytest": { "version": "==8.0.0" } }
        }"#;
        let ctx = parse_pipfile_lock(pip);
        assert_eq!(ctx["dep:pip/requests"], "2.32.3");
        assert_eq!(ctx["dep:pip/pytest"], "8.0.0");

        let poetry = r#"
[[package]]
name = "httpx"
version = "0.27.0"
"#;
        assert_eq!(parse_poetry_lock(poetry)["dep:pip/httpx"], "0.27.0");
    }
}
