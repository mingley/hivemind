//! Multi-tenant store layout for the shared service (M5).
//!
//! One partition (log) per tenant under `{root}/tenants/{tenant_id}/`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Validate a tenant id used as a single path component.
pub fn validate_tenant_id(id: &str) -> Result<()> {
    let id = id.trim();
    if id.is_empty() || id.len() > 128 {
        bail!("tenant id must be 1-128 characters");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        bail!("tenant id `{id}` may only contain [A-Za-z0-9._-]");
    }
    if id == "." || id == ".." {
        bail!("invalid tenant id");
    }
    Ok(())
}

/// `{root}/tenants/{tenant_id}`
pub fn tenant_dir(root: &Path, tenant_id: &str) -> Result<PathBuf> {
    validate_tenant_id(tenant_id)?;
    Ok(root.join("tenants").join(tenant_id.trim()))
}

pub fn ensure_tenant_dir(root: &Path, tenant_id: &str) -> Result<PathBuf> {
    let dir = tenant_dir(root, tenant_id)?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn list_tenants(root: &Path) -> Result<Vec<String>> {
    let base = root.join("tenants");
    if !base.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if validate_tenant_id(&name).is_ok() {
                out.push(name);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_path_traversal_tenant_ids() {
        assert!(validate_tenant_id("../x").is_err());
        assert!(validate_tenant_id("acme/prod").is_err());
        assert!(validate_tenant_id("acme.prod").is_ok());
        let root = tempdir().unwrap();
        let dir = ensure_tenant_dir(root.path(), "acme").unwrap();
        assert!(dir.ends_with("tenants/acme"));
        assert_eq!(list_tenants(root.path()).unwrap(), vec!["acme".to_owned()]);
    }
}
