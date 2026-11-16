//! Server-owned, deny-by-default tenant grants. Clients cannot grant themselves access.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::oauth::AuthIdentity;
use crate::tenant::validate_tenant_id;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TenantAccess {
    pub grants: BTreeMap<String, BTreeSet<String>>,
}

impl TenantAccess {
    /// Without a policy, only the configured confidential client can use the
    /// default tenant. An explicit policy replaces this implicit grant.
    pub fn load(path: Option<&Path>, client: &str, tenant: &str) -> Result<Self> {
        validate_tenant_id(tenant)?;
        let policy = match path {
            Some(path) => serde_json::from_slice::<Self>(
                &std::fs::read(path)
                    .with_context(|| format!("read tenant policy {}", path.display()))?,
            )
            .context("parse tenant access policy")?,
            None => Self {
                grants: BTreeMap::from([(client.to_owned(), BTreeSet::from([tenant.to_owned()]))]),
            },
        };
        for (principal, tenants) in &policy.grants {
            if principal.trim().is_empty() {
                bail!("tenant policy contains an empty principal");
            }
            for tenant in tenants {
                validate_tenant_id(tenant)?;
            }
        }
        Ok(policy)
    }

    pub fn allows(&self, identity: &AuthIdentity, tenant: &str) -> bool {
        identity.bootstrap
            || self
                .grants
                .get(&identity.principal)
                .is_some_and(|g| g.contains(tenant))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_deny_other_clients_and_tenants() {
        let policy = TenantAccess::load(None, "agent", "default").unwrap();
        let agent = AuthIdentity {
            principal: "agent".into(),
            bootstrap: false,
        };
        assert!(policy.allows(&agent, "default"));
        assert!(!policy.allows(&agent, "other"));
        assert!(!policy.allows(
            &AuthIdentity {
                principal: "new-dcr".into(),
                bootstrap: false
            },
            "default"
        ));
        assert!(policy.allows(
            &AuthIdentity {
                principal: "bootstrap".into(),
                bootstrap: true
            },
            "other"
        ));
    }
}
