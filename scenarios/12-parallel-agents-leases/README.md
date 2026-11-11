# Parallel agents, overlapping work, crash recovery

## Story

Two agents get tasks touching `src/parser.py`. Coordination uses leases (TTL)
and change notifications. Agent 1 crashes holding a lease on the parser module.

## Success criteria

- No duplicated implementation; no lost edits; clean merge; tests pass.
- Lease reclaimed within TTL+ε; Agent 2 notified of Agent 1's partial changes.
