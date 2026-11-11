# 05: Evaluation and benchmarking plan

Status: v0.2 (2026-11-10). This plan measures **memory quality**,
**system performance**, and **token efficiency and total cost** *together*.
No single metric is optimized in isolation. Evidence labels follow
[01](01-problem-and-goals.md#evidence-labels).

## Principles

1. **Use the same model, tasks, starting state, tool budget, and
   token-accounting boundary** for every condition. Any deviation is
   reported next to the result.
2. **Keep retrieval quality separate from task success.** A system can
   retrieve perfectly and still not help, or retrieve poorly and still not
   hurt. Both are measured, at different layers.
3. **Count every attributable token.** That includes write-path extraction,
   consolidation, embeddings, tool schemas, retrieved context, follow-up
   expansion, and the agent's whole run. The headline is **cost per
   successful task**, not tokens per retrieval.
4. **Repeat, pair, and report variance and failures.** Agent runs are
   nondeterministic.
5. **Pre-register** each experiment's hypotheses, primary metrics, and
   stop/go thresholds in the repository before running it.
6. **Make everything inspectable:** scenarios, gold answers, gold evidence,
   configurations, raw logs, and token ledgers.

## Four evaluation layers

| Layer | What runs | Answers | Cost per run | Iteration speed |
|---|---|---|---|---|
| **E-R: Retrieval** (offline) | Queries against a fixed memory corpus, with no LLM except embeddings | Does the right memory come back, current and applicable, with its provenance? | ~0 | Seconds |
| **E-QA: Question answering** | An LLM answers from the retrieved memory, scored programmatically or by a validated judge | Is the retrieved content *usable*, including uncertainty and conflicts? | Low | Minutes |
| **E-T: Task** (end to end) | A coding agent completes a sandboxed task with memory tools | Does memory improve success, steps, time, and **cost per successful task**? | High | Hours |
| **E-S: System** | Load, footprint, and crash tests on the service | Is it fast, lean, and durable enough at each scale and deployment mode? | ~0 tokens | Minutes |

Cheap layers run on every change. E-T runs at milestones and for the
falsification experiments ([07](07-falsification-experiments.md)).

## Datasets

### D1: coding-agent memory scenarios (to be built; working name "hivebench")

No public benchmark covers *multi-agent, multi-session shared memory for
coding with updates, contradictions, and poisoning*
([research/05](../research/05-research-and-benchmarks.md)), so we build a
small, inspectable one. Each scenario is a directory:

```text
scenarios/<id>/
  repo.bundle            # small fictional repository (git bundle) with a real build and tests
  timeline.yaml          # prior sessions: who did what, in which order, on which branch or commit
  memories.gold.yaml     # the claims an ideal memory would hold after the timeline, with evidence links
  probes.yaml            # later tasks and questions, each with context fingerprint and checker
  checkers/              # programmatic success checks (tests pass, file diff predicates, forbidden actions)
  README.md              # the story, what changes, what success means
```

**Two modes** separate the write path from the read path:

- **Given memories:** the corpus is pre-populated with the gold claims, plus
  distractors (near-duplicates that don't apply, superseded versions, noise).
  This isolates retrieval and use.
- **Earned memories:** agents actually run the timeline's prior sessions and
  must write their own memories. This tests the write path, and its cost,
  too.

**Scenario families** seed the first 20–40 scenarios; details are in
[research/04](../research/04-coding-agent-memory-practice.md):

| Family | What changes later | What the probe checks |
|---|---|---|
| Recurring build or test failure | A different machine or agent hits the same error | Fix applied, with fewer steps and tokens |
| Dependency incompatibility | Upgrade makes the warning inapplicable on `main` but not on the release branch | Applies the warning only where it holds |
| Architectural decision and reversal | A decision is superseded with new evidence | Explains the *current* decision and its history; honors it in code |
| Handoff of partial work | Agent A stops mid-task; agent B continues | Completes without redoing A's investigation; no duplicated or conflicting work |
| Environment or toolchain gotcha | OS, toolchain, or CI image differs | Uses only applicable knowledge |
| Flaky versus deterministic failure | A second agent contradicts the first | Fixes the root cause, not a retry |
| Superseded API usage | Internal API renamed or deprecated | Uses the new API; cites the change |
| Cross-repository knowledge | Lesson learned in repo X matters in repo Y | Transfers when applicable, doesn't when not |
| Preference adherence | Maintainer preference recorded in another session | Follows it, such as commit style or test layout |
| Dead ends | Prior agent recorded failed approaches | Avoids them |
| Poisoned or incorrect memory | Untrusted content tries to plant an instruction | Doesn't execute it; flags it |
| No relevant memory | Nothing useful exists | Abstains quickly; low token overhead |

**Scale and noise:** each scenario's corpus is also embedded in a background of
unrelated memories at 1k, 10k, 100k, and 1M items. This shows how quality and
latency degrade with corpus size.

**Contamination control:** repositories are fictional and generated, with
randomized names. The no-memory condition (B0) confirms that probes cannot be
solved from model knowledge alone.

### D2: public benchmarks, for comparability

Public benchmarks help us compare against published numbers, but none of them
models our setting. Sizes, licenses, and known validity issues are in
[research/05](../research/05-research-and-benchmarks.md#3-benchmark-matrix).
We reuse them in tiers by cost
([research/05 §5.4](../research/05-research-and-benchmarks.md#54-what-to-reuse-prioritized)):

| Tier | When | Benchmarks | What they tell us |
|---|---|---|---|
| **A** (component) | Every change, nightly | LongMemEval-S (cleaned): knowledge-update, temporal, and abstention subsets; MemoryAgentBench FactConsolidation; STALE; a HaluMem subset (CC BY-NC-ND, internal use only); LoCoMo as a regression check only | Update semantics, staleness, abstention, write fidelity |
| **B** (agentic) | Weekly | MemoryArena; LongMemEval-V2 subsets (workflows and "gotchas"); Context-Bench; MEMTRACK; Evo-Memory stream subset | Memory used across sessions to act |
| **C** (coding) | Milestones | **VibeMemBench** and **MemDev-Bench** (the core); SWE-Bench-CL sequences; the Handoff-Debt protocol; DreamBench-SWE; a Terminal-Bench 2.1 continual split; fresh SWE-rebench or SWE-Bench Pro tasks | Downstream coding outcomes and cost; contamination control |
| **D** (security) | Milestones | GateMem; CoMemBench; GroupMemBench; MPBench; MINJA-, eTAMP-, and rule-file-style attacks adapted to coding | Leakage, isolation, poisoning (F12) |

Rules:

- Pin dataset versions, judge models and prompts, scoring targets, harness
  versions, and time limits.
- Invite vendors to review the configurations of their systems, given the
  history of benchmark disputes
  ([research/03](../research/03-memory-frameworks.md#24-benchmark-disputes-and-replications)).

### D3: synthetic performance corpora

Generated claims, episodes, and artifacts follow realistic size distributions:

- claims: 200 B to 2 KB;
- episodes: up to 10 KB;
- artifacts: 1 KB to 1 MB, log-normal.

Corpora are built at 10k, 100k, 1M, and 10M claims, with query logs sampled
from D1's query shapes (error strings, identifiers, natural-language "why"
questions, filtered lookups).

## Metrics

### Memory quality (E-R and E-QA)

| Metric | Definition |
|---|---|
| Recall@k, Precision@k, nDCG@10, MRR | Against the gold relevant claims for each query |
| **Stale-hit rate** | Share of top-k results that are superseded, invalidated, retracted, or not applicable to the query's context, when the query asks for current knowledge. Lower is better |
| **Applicability precision** | Share of returned claims whose conditions match the context fingerprint |
| **Conflict surfacing** | When gold marks a conflict, the share of queries returning both sides with labels |
| **Detail preservation** | Share of gold critical details (exact error string, version, command, flag) present in what was returned at the level the agent read |
| **Attribution accuracy** | Share of returned claims whose evidence links resolve and actually support the claim. Programmatic where possible; otherwise an LLM judge audited by humans |
| **Abstention quality** | For queries with no relevant memory: the rate of correctly returning nothing or "unknown", and the false-abstention rate |
| **Multi-hop accuracy** | Accuracy on "why" and "what led to" questions that need two or more linked claims (decision chains, supersession) |
| **QA accuracy** | Exact match for structured answers; a rubric otherwise |

### Downstream task outcome (E-T)

| Metric | Definition |
|---|---|
| Success rate (pass@1) | Objective checker passes |
| **pass^k** | All *k* repeated runs pass, a reliability measure |
| Correctness beyond pass | No forbidden actions (for example, ran the poisoned command or added retries for a timezone bug); no unrelated edits |
| Steps and wall time | Tool calls and seconds to completion or give-up |
| **Harm rate** | Share of tasks that pass without memory but fail with it |

### Token efficiency and total cost (E-T, using a token ledger)

Every model call is tagged `{run, scenario, condition, actor, category}`:

- **actors:** `agent`, `memory_writer`, `consolidator`, `embedder`, `judge`;
- **categories:** `extraction`, `dedup_update`, `consolidation`,
  `query_formulation`, `query_embedding`, `write_embedding`,
  `retrieved_context`, `followup_expansion`, `tool_schema`, `agent_other`,
  `background`.

| Metric | Definition |
|---|---|
| **Tokens per successful task** | All non-judge tokens across all attempts ÷ successful tasks |
| **Cost per successful task** | The same in dollars, using a dated price table, with cached and uncached input priced separately |
| Memory-attributable tokens | The sum of the memory-related categories, including **tool-schema tokens added to every turn** by memory tools |
| Write-path cost | Tokens (and local CPU-seconds) to create, deduplicate, update, and consolidate memories, per memory and per scenario |
| **Break-even reuse count** | The number of later uses at which tokens saved exceed the cost of writing and maintaining a memory |
| Local compute | CPU-seconds and peak RSS of hivemind and the embedder per scenario ("per unit of compute") |

**Two accounting subtleties change conclusions, so both are reported:**

- **Prompt caching.** An always-loaded notes file sits in a cacheable prefix
  and can cost far less than its raw token count suggests. Retrieved content
  that varies per turn may break the cache. We report raw tokens *and*
  cache-adjusted cost.
- **Amortization.** Write-path costs are paid once and benefits accrue over
  reuse. We report per-task numbers *and* totals over each scenario's whole
  timeline.

Judge tokens are reported separately and excluded from system cost.

### System performance (E-S)

| Metric | Conditions |
|---|---|
| Throughput (ops/s) | Per operation type |
| Latency p50, p95, p99, p99.9 | Concurrency 1, 4, 16, 64, 256; read/write mixes 100/0, 95/5, 80/20, 50/50; local Unix socket versus remote gRPC and MCP over HTTP |
| Index freshness | Time from write acknowledgement until a write is searchable (lexical, vector) |
| Footprint | Idle and peak RSS, CPU %, disk bytes per memory (data, indexes, vectors, artifacts), write amplification |
| Startup | Cold (empty page cache) and warm, to ready, at each corpus size |
| Recovery | Time to ready after `kill -9` at each corpus size |
| Durability | Zero lost acknowledged writes; no corruption; projections consistent with the log, under process kills, simulated power loss, disk-full, and I/O errors |
| Storage growth | Bytes per 1k memories over a simulated year, including supersession history |

Method notes:

- Use an open-loop load generator that records intended start times (as
  wrk2 does), so results don't suffer from coordinated omission.
- Use HDR histograms.
- Simulate loss of un-fsynced data with a fault-injecting filesystem such as
  LazyFS
  ([research/06](../research/06-architecture-distribution-protocols-security.md)).
- Report hardware, OS, filesystem, build profile, configuration,
  **durability settings** (fsync and synchronous mode, group commit), corpus,
  and embedding model with every number.
- Never rank systems using numbers from incomparable configurations.

## Baselines and conditions

| ID | Condition | Notes |
|---|---|---|
| **B0** | No persistent memory | Lower bound; contamination check |
| **B0f** | Memory off, but agents may write notes to files | Agents do this anyway; "the filesystem is memory" ([research/05](../research/05-research-and-benchmarks.md#29-memory-security)) |
| **B1a** | Prior transcripts in context, truncated to a budget, most recent first | "Context stuffing" |
| **B1b** | Curated notes file, always loaded (`AGENTS.md` style), maintained by the same agents in earlier sessions | Today's most common practice |
| **B1c** | git-markdown memory (`MEMORY.md` index plus topic files) with grep, plus a Beads-style task ledger | The baseline independent studies say to beat ([research/04](../research/04-coding-agent-memory-practice.md#1-summary)) |
| **B2a** | Raw session index with no LLM: funes (vector, BM25, reranker over past sessions) | The strongest existing LLM-free episodic baseline ([research/01](../research/01-agent-memory-systems-rust.md#39-huggingfacefunes)) |
| **B2b** | SQLite full-text search over chunked logs, and basic hybrid RAG | A plain FTS baseline scored 50.0% against 46–53% for 29 memory systems on MemDev-Bench ([research/03](../research/03-memory-frameworks.md)) |
| **B3** | Existing memory systems: Mem0, Graphiti, Letta, Hindsight | Vendor-recommended configurations, pinned; extraction model aligned with ours as a second run |
| **H** | hivemind | With ablations: no applicability conditions, no supersession, full detail instead of cards, lexical only, vector only, no trust tiers, no boundary episodes |
| **O** | Oracle: gold memories or verified experience injected directly | The headroom bound; also "oracle retrieval with hivemind formatting" |

**Fairness rules:**

- Use identical model versions and decoding settings.
- Use an identical system prompt except for a minimal, equivalent memory
  instruction per condition, and report that instruction.
- Use identical turn and tool budgets, sandboxes, scenario order, and seeds.
- Count each condition's own tool-schema tokens.

## Agent harnesses

Two kinds of harness are needed:

- **A controlled harness:** a small open agent loop, such as a minimal
  SWE-agent-style loop. It gives exact per-call token accounting and full
  control of prompts.
- **At least one real product agent connected over MCP,** such as Claude Code
  or Codex CLI, for ecological validity. Its token usage comes from its own
  logs or OpenTelemetry export.

Results must agree in direction across both before we draw conclusions.

## Statistics

- **Seeds:** at least 3 per (task, condition), and 5 for headline claims.
  Add runs where confidence intervals are too wide to decide a
  pre-registered threshold.
- **Paired analysis:**
  - Analyze paired differences per task, with a cluster bootstrap by
    repository or clustered standard errors.
  - Use McNemar's test for success and Wilcoxon signed-rank tests for tokens
    and time.
  - Apply Holm correction across baselines.
  - Report bootstrap 95% confidence intervals for every headline number.
- **Power:** detecting a 3-point paired difference at about 50% base success
  needs several hundred task pairs **[Inf]**
  ([research/05 §5.5](../research/05-research-and-benchmarks.md#55-statistical-protocol)).
  So pool tasks across benchmarks, and treat step and token reductions,
  which have larger effects, as co-primary outcomes.
- **Primary endpoints:**
  - success rate;
  - tokens per successful task;
  - the Pareto frontier of the two, following cost-aware agent evaluation
    ([research/05](../research/05-research-and-benchmarks.md#210-diagnostic-and-evaluation-methodology-papers)).
  - **Memory-induced harm**, reported next to every gain.

  Everything else is secondary and labeled as such.
- **Judges:**
  - Prefer executable or exact-match grading.
  - When an LLM judge is unavoidable, use several judges, avoid judging a
    model with its own family, and human-label at least 100 items, reporting
    agreement (Cohen's κ).
- **Contamination:** use fresh tasks, held-out repositories, and temporal
  filters so memory never holds information from after the task.
- **Failures:** keep a taxonomy:
  - retrieval miss;
  - stale hit;
  - ignored memory;
  - misapplied memory;
  - tool misuse;
  - budget exhaustion;
  - environment failure.

## Reporting template

Every experiment report in `experiments/<id>/REPORT.md` has these sections:

1. Hypothesis and pre-registered thresholds.
2. Setup: hardware, OS, model versions, configuration, durability settings,
   datasets, seeds, and price table date.
3. Results: tables, Pareto plot, and per-scenario breakdown.
4. Failures and threats to validity.
5. A decision: continue, change, or stop.
6. Links to raw logs and ledgers.

## Budget sketch

Token volumes, not prices, are the stable unit. Prices are applied from a
dated table at report time.

| Phase | Shape | Rough agent-token volume [Inf] |
|---|---|---|
| M2 headroom and baselines | 20 scenarios × 5 conditions × 3 runs × ~150k tokens | ~45M |
| M3 ablations | 30 scenarios × 7 conditions × 5 runs × ~150k | ~160M |
| Tier C coding milestone | Stratified 100–150 tasks × the baseline ladder × 3 seeds | Expensive: a published agent leaderboard averaged about $1.84 per rollout **[C]**, so a full 10k-rollout ladder is roughly $10k–30k ([research/05](../research/05-research-and-benchmarks.md#55-statistical-protocol)) |

The plan starts with the smallest phase on purpose: development uses
stratified slices, and full sets run only at milestones
([07](07-falsification-experiments.md)).
