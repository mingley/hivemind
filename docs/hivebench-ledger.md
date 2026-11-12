# hivebench controlled token ledger

Status: implemented (HCP-27). Categories follow
[05-evaluation-plan.md](05-evaluation-plan.md). This is the **controlled
harness** path: a deterministic agent loop over scenario probes with exact
per-call tagging. It does **not** call paid LLMs or the embeddings worker
(`vector.rs` / `model2vec`).

## How to run

From the repository root:

```bash
# One scenario (human-readable category breakdown)
cargo run -p hivebench -- ledger --condition hivemind 01-recurring-ci-failure

# Alias kept for older scripts
cargo run -p hivebench -- dry-run --condition hivemind 01-recurring-ci-failure

# All scenarios, JSON RunRecord (includes full ledger entries)
cargo run -p hivebench -- ledger --condition hivemind --json

# Skip L1 show expansion
cargo run -p hivebench -- ledger --condition hivemind --no-expand 01-recurring-ci-failure

# Baselines
cargo run -p hivebench -- ledger --condition b0 01-recurring-ci-failure
cargo run -p hivebench -- ledger --condition b1b 01-recurring-ci-failure
```

Library entry points: `hivebench::run_controlled` / `hivebench::dry_run`
(`crates/hivebench/src/harness.rs`).

## What one turn does

For each probe:

1. Charge **tool_schema** once from the condition’s `tools/list` JSON
   (hivemind MCP tools for H/B2; smaller surfaces for B1b/B1c/Oracle; none for B0).
2. Charge **query_formulation** from the probe query text (skipped for B0).
3. Run the memory path for the condition (notes file, grep, oracle inject, or
   store `recall`).
4. Charge **retrieved_context** from the concrete text the agent would see
   (rendered L0 cards, notes, grep hits, or gold inject).
5. For H/B2: charge **query_embedding** (estimated) and optional **followup_expansion**
   from a real `show` / L1 JSON of the top card.
6. Once per run: charge **write_embedding** (estimated) over gold (+ distractors
   for H/B2) statements materialized into the corpus.

Offline E-R metrics are still computed and attached to the `RunRecord`.

## Measured vs estimated

| Category | Source | How counted |
|---|---|---|
| `tool_schema` | **Measured** | `chars÷4` over serialized tools/list JSON mirroring `crates/hivemind/src/mcp.rs` |
| `query_formulation` | **Measured** | `chars÷4` over probe query string |
| `retrieved_context` | **Measured** | `chars÷4` over `render_card` output (or notes / grep / oracle text) |
| `followup_expansion` | **Measured** | `chars÷4` over L1 `ClaimDetail` JSON from `Store::get` |
| `query_embedding` | **Estimated** | `chars÷4` of query text; **does not** call `vector.rs` / model2vec |
| `write_embedding` | **Estimated** | `chars÷4` of claim statement; same — no embedder invoke |
| `extraction` / `dedup_update` / `consolidation` / `background` | — | Not charged in given-memories controlled loop (no LLM write path yet) |
| `agent_other` / `judge` | — | Reserved; unused here |

The `chars÷4` heuristic matches `hivemind_engine::recall::estimate_tokens` (the
engine’s own card budget estimator). Each ledger entry carries
`source: measured | estimated`.

## What this proves / does not prove

**Proves:** memory-attributable categories can be attributed from a controlled
loop with concrete schema and retrieval text; B0 has zero memory overhead;
H/B2 schema mass is the real MCP tool surface, not a stub constant.

**Does not prove:** end-to-end paid LLM agent cost, tiktoken/cl100k billed
tokens, or embedding API bills. Earned-memories extraction cost and product
agents over MCP remain separate (see [05](05-evaluation-plan.md) harnesses).
