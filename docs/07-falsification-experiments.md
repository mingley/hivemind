# 07: Experiments that could disprove the approach early

Status: v0.2 (2026-11-10). The experiments are ordered by **information per
unit of cost**: the ones that need little or no hivemind code come first.
Methods and metrics come from [05](05-evaluation-plan.md). Each experiment is
pre-registered in `experiments/<id>/PLAN.md` before it runs.

**Since the build decision** ([04](04-architecture-and-decisions.md#d1-one-integrated-all-rust-system)),
these experiments no longer decide *whether* hivemind exists. They decide what
it emphasizes, which features are on by default, and which design bets to
drop. A negative result removes or reshapes a feature; it is never quietly
ignored.

## Summary

| # | Question | Would disprove | Needs hivemind code? | Relative cost |
|---|---|---|---|---|
| **F1** | Does any memory help coding agents on realistic tasks? | Memory injection on by default | No (oracle versus none) | Low |
| **F2** | Do simple baselines already capture most of the benefit? | Where hivemind adds value beyond notes files | No | Low to medium |
| **F3** | Do agents actually use memory tools well (pull), or must memory be pushed? | The MCP-tool-first interface | No | Low (comes with F2) |
| **F4** | Is lexical search enough for coding memory? | The need for embeddings in the lean default | No (offline) | Very low |
| **F5** | Does the bitemporal and applicability model beat recency sorting? | Model complexity | A paper prototype only | Low |
| **F6** | Are agent-authored writes as good as LLM extraction, and cheaper? | The cheap write path | A thin prototype | Medium |
| **F7** | Do progressive cards beat full detail once follow-ups are counted? | The progressive-disclosure contract | A thin prototype | Medium |
| **F8** | Does our engine meet the latency, footprint, and startup targets at 1M+ memories? | The v0 engine shape (in-memory projections) | Benchmark harness | Low (no tokens) |
| **F9** | What does the daemon and shim cost per agent session? | The cold-start and RSS budget | Benchmark harness | Very low |
| **F10** | Do multi-hop questions need a graph engine? | "No graph database in v0" | Prototype | Medium |
| **F11** | Does sharing across agents help more than it harms? | Default shared scopes | Prototype | Medium |
| **F12** | Do trust tiers actually stop memory poisoning? | The trust model | Prototype | Medium |
| **F13** | Do acknowledged writes survive crashes? | The log implementation (blocks release) | Engine tests | Very low |

## F1: Headroom (does memory matter at all?)

- **Hypothesis:** with **perfect** memory (oracle O: gold claims in context),
  coding agents complete the D1 scenarios with higher success *or* materially
  fewer tokens per successful task than with no memory (B0).
- **Method:** 20 scenarios in given-memories mode, B0 versus O, 3 runs each,
  on the controlled harness with one strong model.
- **Disproved if:** O improves success by fewer than 10 points **and** reduces
  tokens per successful task by less than 20%.
- **If disproved:** narrow the default. Keep the scenario families with
  headroom, such as handoffs, cross-machine reuse, and stale-memory
  detection, and make memory injection opt-in elsewhere.
- **Why first:** if a perfect memory doesn't pay on a scenario family, no real
  memory will. It costs a few days, needs no hivemind code, and also validates
  the scenario set and checkers.

## F2: Simple baselines (do we need a new system?)

- **Hypothesis:** the best of B1b (always-loaded notes), B1c (markdown plus
  grep), and B2 (vector RAG) captures **less than 70%** of O's gain on
  scenarios with updates, contradictions, cross-agent reuse, or handoffs.
- **Method:** the F1 scenarios plus earned-memories mode for B1b and B1c,
  3–5 runs each.
- **Disproved if:** a simple baseline reaches at least 85% of O's gain at no
  more than 1.2× its tokens per successful task.
- **If disproved:** support the simple approach directly. Export to and from
  git-markdown memory, and focus hivemind's defaults on what the baselines
  lack: cross-tool and cross-machine sharing, staleness detection, conflicts,
  and handoffs.
- **Why:** published results suggest file- and grep-based agents can be
  competitive on some memory benchmarks
  ([research/03](../research/03-memory-frameworks.md),
  [research/05](../research/05-research-and-benchmarks.md)). Independent
  coding studies found existing memory systems rarely beat no memory
  ([research/04](../research/04-coding-agent-memory-practice.md#7-pain-points-and-failure-modes-of-todays-approaches)).
  The git-markdown baseline is the one to beat.

## F3: Pull versus push (do agents use memory tools?)

- **Hypothesis:** agents given memory tools (pull) query them at the right
  moments (task start, new errors, before decisions) often enough that pull
  matches or beats harness-injected memory (push: automatic retrieval at
  session start and on error output).
- **Method:** instrument F2's B1c and B2 runs. Add push variants that use
  harness hooks. Measure the **missed-opportunity rate**: the share of probes
  where a relevant memory existed but was never retrieved.
- **Disproved if:** the missed-opportunity rate under pull exceeds 30%, or
  push wins on success at equal cost.
- **If disproved:** make push via hooks the primary integration, keep
  MCP tools secondary, and invest in automatic triggers such as error
  signatures and file paths.

## F4: Lexical versus semantic versus hybrid retrieval

- **Hypothesis:** for coding memories (error strings, identifiers, versions,
  paths), BM25 is competitive, and hybrid search adds a measurable but modest
  gain on natural-language "why" queries.
- **Method:** offline E-R on D1, plus a held-out set of paraphrased queries.
  Conditions:
  - BM25 alone;
  - a small general embedding model;
  - a code-oriented embedding model;
  - a static (model2vec-style) embedding model;
  - hybrid search with RRF;
  - hybrid search plus a reranker.

  Candidates are chosen from
  [research/07](../research/07-embeddings-and-local-inference.md).
- **Decision rule:** if BM25 reaches at least 95% of hybrid nDCG@10 on D1,
  ship **lexical-only as the default lean mode** with embeddings optional. If
  hybrid wins by 10% or more, embeddings are in the default.
- **Why early:** it decides the default footprint (whether to bundle an
  embedding model) at almost no cost.

## F5: Does the temporal and applicability model earn its complexity?

- **Hypothesis:** the model in [03](03-memory-model.md) (supersession, valid
  and record time, applicability conditions) reduces the **stale-hit rate**
  and raises **applicability precision** compared with plain retrieval sorted
  by recency.
- **Method:** offline E-R on D1 scenarios with injected updates, branch
  differences, and contradictions. The model can be a *paper prototype*: SQL
  views over a SQLite file built by a short script, with no Rust needed.
  Conditions:
  - vector search;
  - vector plus recency;
  - BM25 plus recency;
  - BM25 plus status filter (no conditions);
  - the full model.
- **Disproved if:** recency or status filtering comes within 2 points of the
  full model on stale-hit rate and applicability precision.
- **If disproved:** drop `applies_when` and bitemporal queries from v0. Keep
  only status and supersession.

## F6: Write path (agent-authored versus LLM extraction)

- **Hypothesis:** agent-authored structured writes, with near-duplicate
  candidates returned for the agent to resolve, reach at least the same
  gold-claim coverage and correctness as LLM extraction plus an update phase
  (the Mem0 or Graphiti style), at **under half the write-path tokens**.
- **Method:** earned-memories mode on 15 scenarios. Conditions:
  - agent-authored;
  - Mem0-style extraction (the B3 system, or a faithful reimplementation
    with the same model);
  - raw episode chunks with no extraction.

  Metrics:
  - write tokens per memory;
  - gold coverage;
  - incorrect or hallucinated memory rate;
  - duplicate rate;
  - E-R and E-T outcomes downstream.
- **If disproved:** make LLM extraction an asynchronous, budgeted default
  rather than optional, and redo the cost model.

## F7: Progressive disclosure

- **Hypothesis:** L0 cards with on-demand expansion yield the lowest **tokens
  per successful task**, follow-up calls included, with no loss of success
  compared with L1 full detail and raw top-k chunks.
- **Method:** E-T on 20 scenarios, with the same retrieval and different
  response levels. Count follow-up expansions and their tokens, and record
  prompt-cache effects.
- **If disproved:** default to L1, or adapt the level to the query type,
  for example full detail for handoffs and cards for error lookups.

## F8: Engine sufficiency (does the v0 engine shape hold?)

- **Hypothesis:** the v0 engine meets the targets in
  [01](01-problem-and-goals.md#provisional-targets-hypotheses-to-test-not-commitments)
  at 100k and 1M memories with 64 concurrent clients, and degrades gracefully
  at 10M. The v0 shape is a segmented log, in-memory projections, our own
  BM25, and an exact quantized vector scan.
- **Method:** the E-S workload suite on D3, including replay time, snapshot
  load time, and RSS at each size. A mature reference engine runs the same
  queries, to calibrate retrieval quality and latency.
- **Decision rules:**
  - Replay over 2 s at 1M: add snapshots.
  - RSS over budget: move the lexical and vector indexes to memory-mapped
    immutable segments ([04](04-architecture-and-decisions.md#projections-v0-in-memory)).
  - Vector latency over budget: add ANN for large scopes.
  - Lexical quality below the reference by more than 2 nDCG points: fix the
    tokenizer or scoring before adding features.

## F9: Daemon and shim overhead

- **Hypothesis:** the stdio shim adds under 50 ms to an agent's MCP startup
  and under 10 MB of RSS per session. The shared daemon keeps total memory
  flat as the number of agents grows from 1 to 16.
- **Method:** cold and warm shim start, RSS per session, and daemon RSS
  against the number of concurrent agent sessions. Compare with each agent
  opening the store in-process, as an engineering reference only.
- **Decision rule:** if the shim breaks the budget, optimize the handshake or
  ship a long-lived socket mode for agents that support it. Removing the
  single owner is not an option
  ([04](04-architecture-and-decisions.md#d4-one-owner-per-store)).

## F10: Is a graph engine needed for multi-hop questions?

- **Hypothesis:** search plus bounded expansion over edge tables (1–3 hops)
  answers D1's multi-hop questions (decision chains, "what led to") as well as
  graph-native retrieval does.
- **Method:** E-QA on multi-hop probes. Conditions:
  - hivemind search plus expansion;
  - Graphiti with its hybrid search;
  - a personalized-PageRank variant in the style of HippoRAG over the same
    entities.
- **If disproved** (graph methods ahead by at least 10 points at acceptable
  cost): add a graph projection or engine and record it in a new decision
  record.

## F11: Is sharing a net good?

- **Hypothesis:** project-scoped sharing across agents improves outcomes over
  private memory, and team or global sharing does not raise the harm rate
  when applicability and trust are enforced.
- **Method:** E-T with private, project, team, and global scope settings,
  with cross-project noise at 100k memories.
- **If disproved:** default to narrower scopes and require explicit promotion
  for everything.

## F12: Does the trust model stop poisoning?

- **Hypothesis:** trust tiers and quarantined rendering cut the success rate
  of memory-injection attacks by at least 80% compared with B2 and B3, with no
  meaningful loss on clean tasks.
- **Method:** attacks adapted from the published memory-injection and
  RAG-poisoning literature
  ([research/05](../research/05-research-and-benchmarks.md),
  [research/06](../research/06-architecture-distribution-protocols-security.md)),
  planted through untrusted evidence and through compromised agent writes.
- **If disproved:** require human promotion for any claim that can lead to
  command execution, and revisit the trust model.

## F13: Crash safety

- **Hypothesis:** with the documented durability settings, no acknowledged
  write is lost and projections always rebuild consistently.
- **Method:**
  1. 1,000 `kill -9` cycles during mixed writes.
  2. Power-loss simulation by dropping un-fsynced data with a fault-injecting
     filesystem.
  3. Disk-full and I/O-error injection.
  4. Afterwards, check log and projection invariants.
- **Any failure blocks a release.** This tests the configuration, not the
  premise.

## What negative results would change

- **F1 or F2 negative** (memory gives little over no memory, or over
  git-markdown notes):
  - narrow the default product to what the baselines lack, such as
    cross-tool and cross-machine sharing, verification of stale memories,
    and coordination (handoffs, leases);
  - make memory injection opt-in.

  Independent studies already point this way
  ([research/04](../research/04-coding-agent-memory-practice.md#7-pain-points-and-failure-modes-of-todays-approaches)).
- **F5 negative:** the model shrinks to a scoped, provenance-carrying note
  store with status and supersession only.
- **F6 negative:** hivemind needs a budgeted background extraction service,
  and the "lean local" claim must be restated in token terms.
- **F8 negative:** the engine grows on-disk index segments, snapshots, or ANN
  earlier than planned. These are changes inside the engine, not a reason to
  swap engines.
- **F13 negative:** stop feature work until the log path passes. Durability is
  not traded for speed.
