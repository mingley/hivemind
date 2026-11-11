# M3 implementation notes

Status: **implemented** (2026-11-11). Milestone **M3 Coding-agent semantics**
from [06](06-prototype-plan.md).

## Delivered

| Piece | Where |
|---|---|
| Citation verify (`valid`/`drifted`/`broken`) | `hivemind-core::citation`, `Store::get_with_workspace` |
| Lockfile fingerprint | `hivemind-core::fingerprint`, CLI `--here` / `--workspace` |
| Handoff payload shape | `ClaimData::validate` for `ClaimKind::Handoff` |
| Leases + fencing | `EventBody::Lease*`, `Store::acquire/renew/release_lease`, CLI |
| Default `review_by` | `store::default_review_by` on remember |
| Copilot JSONL + markdown | `hivemind::exchange`, `hivemind import|export` |

## Commands

```bash
hivemind show <id> --here
hivemind remember "…" --cite src/lib.rs:10-12 --workspace .
hivemind lease acquire wi:parser --ttl-secs 7200
hivemind import memories.jsonl --format jsonl
hivemind export -o MEMORY.md --format markdown
```
