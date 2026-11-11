# 08: Hippocampal design: which ideas become mechanisms

Status: v1.0 (2026-11-10). Synthesized from
[research/08](../research/08-computational-hippocampus.md), which verified every
cited paper on that date. Labels follow
[01](01-problem-and-goals.md#evidence-labels).

The rule for this document: **a hippocampal idea enters hivemind only as an
engineering mechanism with a falsifiable experiment.** Biological plausibility
earns nothing. The research found that independent evidence for
hippocampus-inspired AI memory is thin, and mostly negative on real coding
tasks **[I]**
([research/08](../research/08-computational-hippocampus.md#1-summary)).

## What the theory already confirms

hivemind's existing commitments match two broadly accepted frameworks
**[I]**:

- **Complementary learning systems.** A fast episodic store sits beside slow,
  selective consolidation into general knowledge
  ([McClelland et al., 1995](https://doi.org/10.1037/0033-295X.102.3.419);
  updated by Kumaran, Hassabis & McClelland, 2016). In hivemind:
  - episodes and artifacts are appended immediately;
  - claims and procedures are derived slowly and optionally.
- **Hippocampal indexing theory.** The hippocampus stores an *index* that
  reinstates full patterns stored elsewhere
  ([Teyler & DiScenna, 1986](https://doi.org/10.1037/0735-7044.100.2.147)).
  In hivemind, compact cards point to full evidence.

The most useful refinement is **key–value memory** (Gershman, Fiete & Irie,
2025). Retrieval *keys* should be optimized to tell memories apart, and stored
*values* optimized for fidelity. hivemind's cards are keys and its evidence is
values. Keys can be enriched freely (entities, error signatures,
applicability) without touching immutable evidence
([research/08 §A2](../research/08-computational-hippocampus.md#a2-hippocampal-memory-indexing-theory--keyvalue-memory)).

## Mechanisms

IDs follow [research/08 §4.1](../research/08-computational-hippocampus.md#41-mapping-table).
The experiments are defined there and map onto [07](07-falsification-experiments.md).

### Build into v0 (cheap, deterministic, each with a test)

| ID | Principle | hivemind mechanism | Status | Test |
|---|---|---|---|---|
| C1 | Indexing / key–value | Cards are keys; evidence is values. **Key expansion** at write time adds normalized error codes, paths, symbols, `package@version`, and applicability terms to the index, with no LLM | Cards and code-aware tokens: **done (M0)**. Explicit key extraction: M3 | E1: raw text vs card vs card plus keys, recall@5 per token |
| C8 | Concept cells | Canonical entity keys (`pkg:`, `repo:`, `err:`, paths, symbols), an `about` index, entity filters | `about` index: **done (M0)**. Entity boost in ranking: M3 | E8: entity boost on/off |
| C15 | Source monitoring | Mandatory provenance (method, origin, evidence, citations); cards show source and verification; derived claims bottom out in evidence | **Done (M0)** | E15: action rate on unsupported claims with and without source display |
| C6 | Reconsolidation as latent-cause inference | Feedback on retrieved claims. Helpful with evidence is a validated use; contradiction makes a claim *disputed*; supersede only when the context matches, otherwise a sibling claim | Feedback and disputed state: **done (M0)**. Context-match rule for supersede: M3 | E6: stale-application repeats with and without feedback |
| C14 | Adaptive forgetting | Accessibility decays by kind and is reinforced by validated use; **decay never deletes** | Recency by kind and feedback multipliers: **done (M0)**. Activation in the style of ACT-R: M3 | E14: stale hits vs recall of rare but valid claims |
| C5 | Pattern separation | Near-duplicates in the same scope are surfaced at write time. Ones that differ by context stay separate, with the distinguishing conditions shown | Similar claims on write: **done (M0)**. A `contrasts_with` edge and "differs in" display: M3 | E5: wrong-variant application rate |
| C2 | Event segmentation | Episodes cut at deterministic boundaries: session start and end, work-item or branch change, commit, tests flipping fail→pass, cwd change, idle gap. A write prompt fires at each boundary | M1, with episodes and hooks | E2: fixed chunks vs whole session vs boundary episodes |
| C9 | Temporal contiguity | `expand(card, neighbors=±k, forward)` over the ordered log; small boost for neighbors of strong hits | M1, nearly free because the log is ordered | E9: does forward expansion surface the fix? |
| C3 | Novelty and prediction error | Salience *weights* ranking and retention: non-zero exit, test failure, first-seen error signature, contradiction. **It never gates writes** | M3 | E3: pure surprise gating should lose "boring but critical" facts |
| C18 | Selective retrieval | Abstain below a score threshold; push only at boundaries | M3 | Irrelevant-injection rate and tokens (F3, F7) |

### Test in v1, in priority order

| ID | Principle | Mechanism | Why it waits |
|---|---|---|---|
| C13 | Generalization-optimized CLS (Go-CLS) | Promote a consolidated claim only if it predicts outcomes on held-out episodes | Consolidating everything follows an inverted U and ends below no memory, as reported in one 2026 agent study **[C]** |
| C7 | Prioritized replay (gain × need) | Consolidation queue under a token budget. Priority = gain (contradictions, new failures, unconsolidated clusters) × need (activation of the entities involved, open work items) ÷ cost | Needs consolidation jobs first; sleep-time compute helped software engineering only at low budgets **[C]** |
| C4 | CA3 pattern completion | Personalized PageRank over the entity–claim–episode graph from partial cues (`expand=assoc`) | Helps multi-hop, hurts simple facts **[I]**; route by query type (F10) |
| C16 / C17 | Generative consolidation vs retrieval-time aggregation | Summaries and procedures as "cortex" vs aggregation cards built on demand | Rewriting collapses detail; must be measured on detail questions |
| C19 | Factorized structure (Tolman–Eichenbaum Machine) | `procedure` claims with typed slots (`{pkg}`, `{version}`, `{os}`) | Needs enough procedures to transfer |
| C10 | Successor representation | Counts of the form "after error class X in context C, Y succeeded k of n times" | Needs at least about 5 prior episodes per class |
| C12 | Schemas | Fast-track claims that fit known patterns; novel claims start provisional | Risk of confirmation bias |
| C11 | Engram linking | Weak `co_temporal` edges within one agent's episode or work item | Interleaved multi-agent timing is not relatedness |

### Rejected as metaphor without engineering payoff

These are recorded so they aren't re-proposed without new evidence
([research/08 §4.3](../research/08-computational-hippocampus.md#43-rejected-as-metaphor-without-engineering-payoff)):

- Hopfield, Kanerva-style sparse distributed memory, or dense associative
  memory as the store;
- test-time weight memory (Titans, Larimar);
- generative replay;
- random forgetting in the style of neurogenesis;
- literal sleep cycles;
- generative reconstruction of evidence;
- emotion analogues;
- pure surprise gating;
- deleting episodes after consolidation.

## Risks of over-applying the analogy

- **Contested theory.** Human reconsolidation, replay-as-planning, and the
  exact form of consolidation are debated. A mechanism must earn its place on
  hivebench ([05](05-evaluation-plan.md)).
- **Losing what matters.** Surprise gating or aggressive decay drops dull but
  critical facts, such as build commands. Hence weighting, never gating, and
  decay that never deletes.
- **Distortion.** Consolidation and schemas introduce gist errors and
  confirmation bias. Consolidated claims never outrank their evidence, and
  taint propagates (D6, [03](03-memory-model.md#provenance-verification-and-trust)).
- **Amplifying poison.** Spreading activation, co-temporal links, and
  salience boosts can amplify a poisoned memory. Trust never exceeds the
  weakest edge.
- **One brain, many agents.** "Surprise" and "close in time" are per agent or
  per work item, never global across interleaved agents.
