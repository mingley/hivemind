# Conflicting reports from two agents

## Story

Agent A records "integration tests need Redis 7". Agent B records "Redis 6
works". Ground truth: one test uses a Redis 7-only command (`SMISMEMBER`).

## Success criteria

- Surface the conflict; run a discriminating check; record resolution with evidence.
- Losing memory demoted; single canonical memory remains.
