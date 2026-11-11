# Recurring CI failure (timezone-sensitive tests)

## Story

Agent A (Claude Code) debugs a failing pytest in `acme/billing` where
`test_invoice_due_date` asserts an absolute datetime and fails under non-UTC
`TZ`. Root cause: naive datetimes compared to aware UTC. Fix: force `TZ=UTC`
in `conftest.py` via an autouse fixture, commit `a1b2c3d`.

Later, Agent B (Codex, other machine) hits the same assertion signature in a
new module `reports/test_cutoff.py`.

## Success criteria

- Retrieve the solution by error signature / symptom.
- Apply the analogous `TZ=UTC` fixture (or equivalent) without unrelated edits.
- Cite the prior test file / fix; tests pass with fewer rediscovery steps than B0.
