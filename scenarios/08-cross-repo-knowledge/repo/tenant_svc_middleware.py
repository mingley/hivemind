def require_tenant(headers: dict) -> str:
    tenant = headers.get("X-Tenant") or ""
    if not tenant.strip():
        raise ValueError("400 missing X-Tenant")
    return tenant
