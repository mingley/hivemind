# Evaluation seed scenarios (M2)

Twelve seed scenarios for the hivebench harness. Families and intent come from
[research/04 §9](../research/04-coding-agent-memory-practice.md#9-evaluation-seed-scenarios);
directory contract from [docs/m2-implementation-notes.md](../docs/m2-implementation-notes.md)
and [docs/05-evaluation-plan.md](../docs/05-evaluation-plan.md).

Each directory contains:

| File | Role |
|------|------|
| `README.md` | Story and success criteria |
| `timeline.yaml` | Prior sessions (actor, summary, optional branch) |
| `memories.gold.yaml` | Gold claims (`id`, `kind`, `scope`, `statement`, `about`, `when`, `fields`, `status`) |
| `probes.yaml` | Retrieval probes with `expect.claim_ids` or `expect.abstain` |
| `distractors.yaml` | Optional near-duplicates / noise (not current for the main probe) |
| `repo/` | Optional tiny tree (1–3 files) for citations and checkers |

## Index

| Id | Family | Project scope (examples) |
|----|--------|--------------------------|
| [01-recurring-ci-failure](01-recurring-ci-failure/) | Recurring CI failure (TZ-sensitive tests) | `project:acme/billing` |
| [02-dependency-incompatibility](02-dependency-incompatibility/) | Dependency pin then upstream fix | `project:acme/gateway` |
| [03-architectural-reversal](03-architectural-reversal/) | ADR superseded | `project:acme/shop` |
| [04-handoff-partial-work](04-handoff-partial-work/) | Cross-agent handoff | `project:acme/edge` |
| [05-environment-gotcha](05-environment-gotcha/) | OS-scoped toolchain gotcha | `project:acme/analytics` |
| [06-flaky-then-regression](06-flaky-then-regression/) | Flaky retired, real regression | `project:acme/syncer` |
| [07-superseded-api](07-superseded-api/) | API rename / signature drift | `project:acme/accounts` |
| [08-cross-repo-knowledge](08-cross-repo-knowledge/) | Cross-repo fact + access control | `project:acme/tenant-svc`, `client-sdk` |
| [09-conflicting-reports](09-conflicting-reports/) | Conflicting agent reports | `project:acme/cache-it` |
| [10-poisoned-memory](10-poisoned-memory/) | Prompt-injection poison | `project:acme/payments` |
| [11-stale-convention](11-stale-convention/) | Stale convention vs code | `project:acme/ledger` |
| [12-parallel-agents-leases](12-parallel-agents-leases/) | Parallel agents + lease reclaim | `project:acme/protocol` |

Load with `hivebench` (`load_scenarios("scenarios")`) once the crate CLI is wired.
