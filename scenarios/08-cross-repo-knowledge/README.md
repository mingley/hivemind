# Cross-repo knowledge with access control

## Story

In repo B (`acme/tenant-svc`), agents remember that clients must send
`X-Tenant` or the handler returns 400. Repo A (`acme/client-sdk`) adds a new
call.

## Success criteria

- Agent with read access to B retrieves the header requirement; integration test passes.
- Negative control: agent without access to B must **not** receive B's memory (leakage = hard fail).
