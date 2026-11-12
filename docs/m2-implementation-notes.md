# M2 implementation notes (working)

Status: **implemented** (2026-11-11). Milestone **M2 Evaluation harness** from
[06](06-build-plan.md), using the scenario shape in
[05](05-evaluation-plan.md) and the twelve seeds in
[research/04 §9](../research/04-coding-agent-memory-practice.md#9-evaluation-seed-scenarios).

## Deliverables

1. `crates/hivebench` — scenario schema, loaders, gold ingest, offline E-R
   metrics (Recall@k, stale-hit, applicability precision, abstention).
2. `scenarios/` — twelve seed scenario directories (families 1–12).
3. Controlled harness with a real token ledger (categories from 05) — see
   [hivebench-ledger.md](hivebench-ledger.md) (HCP-27 replaced the dry-run stub).
4. Baselines: B0, B1b (notes file), B1c (git-markdown + grep), B2
   (lexical-only / store BM25 as stand-in), oracle (gold inject).
5. CLI: `hivebench er` (offline retrieval), `hivebench list`, `hivebench
   validate`, `hivebench ledger` (`dry-run` alias).
6. Update [06](06-build-plan.md) with an M2 status section when landed.

## Scenario directory contract

```text
scenarios/<id>/
  README.md
  timeline.yaml
  memories.gold.yaml
  probes.yaml
  distractors.yaml          # optional near-duplicates / noise
  checkers/                 # optional; shell or predicate specs
  repo/                     # optional mini tree (bundle optional)
```

Gold claims use the same fields the engine understands (kind, scope,
statement, about, when, fields). Probes carry `query`, `ctx`, and expected
claim ids or abstention.

## Out of scope for M2

Full LLM agent loops against paid APIs, Tier C coding leaderboards, M3
leases/citation verify, vectors (M4).
