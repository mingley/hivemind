# Research 05: Agent-memory research literature and benchmarks

> Research thread report, checked 2026-11-10. Produced by a research agent from public sources and saved verbatim (delivered in three parts and joined here). Evidence labels: **[V]** verified in a primary source, **[C]** author or vendor claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Synthesis and decisions live in [`docs/`](../docs/).

## Executive summary

**Coverage.** I checked about 150 sources on 2026-11-10: arXiv metadata and abstracts through the arXiv API, targeted sections of the papers' HTML versions, OpenReview for publication venues, GitHub for licenses, and vendor and researcher blogs. A large 2026 literature appeared after my training data, and most of it is unreviewed preprints, so treat those findings as leads rather than settled results.

1. **Simple baselines are hard to beat on conversational memory benchmarks.**
   - In Mem0's own LoCoMo table, feeding the full conversation scores 72.90 on its LLM-judge metric (J), ahead of Mem0 (66.88) and its graph variant (68.44).
   - Letta's agent, with only file search and grep, reports 74.0%.
   - On GroupMemBench, a plain keyword-ranking (BM25) baseline matches or beats most memory systems.
   - An independent cost testbed found plain retrieval matched mem0 at 8.4× lower total cost of ownership.
   - Hivemind has to beat these baselines, not just "no memory".
2. **The bottleneck is choosing what to retrieve under a token budget.** Long-context models lose accuracy as histories grow. One system, SmartSearch, scored 93.5% on LoCoMo and 88.4% on LongMemEval-S with deterministic retrieval plus reranking. It used 8.5× fewer tokens than full context. EvalMem traces most failures to retrieval, not to how memories were stored.
3. **Coding-specific memory evidence is new and sobering.**
   - VibeMemBench: past experience already verified as useful raises resolution by 1.1–4.5 percentage points (pp) and cuts steps. Yet 11 of 12 pairings of a coding agent with an existing memory system fail to beat memory-off.
   - MemDev-Bench: memory can break tasks the agent would otherwise solve when it reuses an old fix without checking it still applies. Raw past trajectories save more steps than compressed memories.
   - AGENTS.md context files add more than 20% inference cost without improving success.
   - Handing interrupted work to another agent with notes cuts tokens by 42–63%.
4. **Experience-based memory on SWE-bench gives small gains, none independently replicated.** Typical results are +3 to +5 pp success with fewer steps (ReasoningBank, Agent KB, SWE-Exp).
5. **Write-time extraction is where cost and errors hide.** Mem0's "90% token savings" counts only the tokens retrieved, not the tokens spent extracting memories. HaluMem shows errors start and pile up during extraction and update.
6. **Updated and stale facts are unsolved.** No method tested on MemoryAgentBench's multi-hop forgetting task scored above 28%. The best model on STALE reaches 55.2%. Models over-trust stale stored facts even when an authoritative tool has the current value.
7. **Graphs help only for complex multi-hop or summary questions.** They don't help simple fact lookups, and they inflate prompts to as much as about 40k tokens.
8. **Shared memory is a lasting attack surface, and no system yet handles shared-memory governance well.**
   - Demonstrated attacks: MINJA, eTAMP, MemPoison, ShadowMerge, misalignment that spreads through memory or plain files, malicious packages planted via rule files, and the SpAIware ChatGPT memory exploit.
   - Governance: on GateMem, no method achieves usefulness, access control and reliable deletion at the same time.
9. **The benchmarks themselves are fragile.**
   - LoCoMo has label errors and conversations short enough to fit in context.
   - LongMemEval was re-cleaned in 2025-09, so results need a pinned version.
   - Choosing which stored form gets credit (the scoring target) can flip system rankings.
   - About 30% of Terminal-Bench 2.0 tasks needed repair, and time limits shift results by up to 20 pp.

**Recommendation.**
- **Test in three layers:**
  - Components: LongMemEval-S, HaluMem, STALE, FactConsolidation.
  - Agentic memory: MemoryArena, LongMemEval-V2, Context-Bench.
  - Downstream coding: VibeMemBench, MemDev-Bench, SWE-Bench-CL, the Handoff-Debt protocol, DreamBench-SWE, Terminal-Bench 2.1.
- **Test governance and security** with GateMem, CoMemBench and MPBench.
- **Always include** a raw-log-plus-grep baseline and an oracle (perfect-memory) baseline.
- **Report costs including write-time tokens,** plot cost against success, use paired multi-seed runs, and report pass^k (success on all k repeated trials).

**Main gap:** no public benchmark covers multi-agent, cross-tool shared coding memory with updates, contradictions, handoffs, access control and poisoning, all checked by executable tests. Section 5.6 of the report sketches one.

---

## Research literature and benchmarks on agent memory: evidence base for hivemind

*All sources checked 2026-11-10. Public sources only.*

**Label convention.**
- **[V]**: a bibliographic or setup fact (existence, venue, size, license, method, how a metric is defined) verified in the cited primary source.
- **[C]**: a result or number that appears in the cited source but is the authors' or vendor's own claim and has not been independently checked.
- **[I]**: an independent or third-party evaluation.
- **[Inf]**: my inference.
- **[?]**: unknown or not verified.

Numbers come from abstracts and targeted sections of each paper's HTML version unless stated otherwise. Full appendices were not audited.

## 1. Summary

1. **Simple baselines match or beat purpose-built memory on today's conversational benchmarks.**
   - In Mem0's own LoCoMo table, full context scores 72.90±0.19 J. Mem0 scores 66.88 and Mem0-graph 68.44 [C][S29].
   - Letta's filesystem-and-grep agent reports 74.0% [C][S42].
   - ConvoMem reports full context at 70–82% versus Mem0 at 30–45% for histories under 150 conversations [C][S120].
   - On GroupMemBench, BM25 matches or exceeds most memory systems [C][S118].
   - An independent testbed found plain retrieval matched mem0 at 8.4× lower total cost of ownership (TCO). Graphiti and cognee trailed at 55–56% versus 77–81% [I][S40].
2. **Long context alone degrades; selection and ranking is the lever.**
   - Commercial assistants drop 37–64% on LongMemEval versus reading the history offline [C][S93].
   - About 300-token focused prompts beat about 113k-token full prompts across 18 models [C][S69].
   - SmartSearch reaches 98.6% recall, but only 22.5% of gold evidence survives naive truncation. With reranking it scores 93.5% (LoCoMo) and 88.4% (LongMemEval-S) at 8.5× fewer tokens than full context [C][S38].
   - EvalMem attributes most failures to retrieval: 22.1%, versus 7.7% encoding and 6.5% generation [C][S123].
3. **Coding: useful experience exists, but current memory systems don't deliver it.**
   - VibeMemBench: injecting execution-verified prior experience raises resolution by 1.1–4.5 pp on 4 of 5 solvers and reduces steps on all 5. Yet 11 of 12 solver × memory-system pairings fail to beat memory-off [C][S112].
   - MemDev-Bench: memory aids localization and editing but causes failures when agents don't reassess whether it still applies. Raw trajectories cut steps more than compressed memories [C][S113].
4. **Experience-based memory gains on SWE-bench are modest and mainly cut steps.**
   - ReasoningBank: 34.2→38.8 resolved (steps 30.3→27.5) and 54.0→57.4 (steps 21.1→19.8) [C][S48].
   - Agent KB: +4.0 pp on SWE-bench Lite [C][S51].
   - SWE-Exp: 73.0% with Claude 4 Sonnet [C][S50].
   - None has been independently replicated [?].
5. **Static always-on context is costly and often useless.** AGENTS.md files don't generally improve success and add more than 20% inference cost [C][S55]. A 288-run ablation finds no measurable correctness effect [C][S56]. Gated injection (SEDM) saves 18–31% of input tokens [C][S62].
6. **Handoffs pay off in efficiency.** Context-bearing handoffs cut median agent events by 20–59% and prompt tokens by 42–63%. Effects on solve rate are small [C][S66]. Model-to-model trajectory transfer carries a "handoff tax" [C][S67].
7. **Within a session, simple observation masking halves cost** versus a raw agent and matches LLM summarization on SWE-bench Verified [C][S73].
8. **Write-time cost and errors are under-reported.**
   - Mem0's token metric counts retrieved context only [V][S29].
   - MemoryTax reports that Mem0's build step costs more than a full-context rollout, and that a stronger reader can't recover what a weaker writer omitted. This evidence is weak: 1 conversation, 1 run per cell [C][S39].
   - HaluMem finds hallucinations originate in extraction and update [C][S97].
   - ACE documents "context collapse" from wholesale rewriting [C][S47].
9. **Updates and staleness are unsolved.**
   - MemoryAgentBench multi-hop selective forgetting: 28% or less for all methods [C][S95].
   - STALE: best model 55.2% [C][S119].
   - GroupMemBench knowledge updates: 27.1% [C][S118].
   - Models answer with the stale stored value 92–100% of the time in the Memory Trust Gap "Benefit" suite [C][S90].
10. **Graphs help selectively.** Basic retrieval (RAG) matches or beats GraphRAG on simple fact retrieval. GraphRAG wins on complex reasoning and summarization, but prompts grow to as much as about 40k tokens [C][S21]. Graph construction needs an LLM extraction pass per passage [V][S17].
11. **Shared memory is a lasting attack and propagation channel.** Demonstrated attacks: MINJA [S78], eTAMP [S85], MemPoison [S84], ShadowMerge (93.8% attack success on Mem0) [S86], misalignment that spreads through memory or plain files [S88], rule-file package hijacking [S89], and SpAIware [S81]. Lifecycle gates reduce attack success to about 1% in one evaluation [C][S87].
12. **Multi-principal governance is unsolved.** On GateMem, no method achieves usefulness, access control and forgetting at once [C][S116]. CoMemBench shows a trade-off between sharing and isolation [C][S117].
13. **Benchmarks are fragile.**
    - LoCoMo flaws [C][S41]; LongMemEval re-cleaned 2025-09 [V][S94].
    - The scoring target flips rankings [C][S124].
    - SWE-bench test, leakage and memorization issues [C][S128][S131][S132].
    - About 30% of Terminal-Bench 2.0 tasks repaired; time limits shift results by 20 pp [C][S108].
    - Vendor-run comparisons conflict (Mem0 vs Zep vs Letta).
14. **Recommended:** evaluate in three layers (components, agentic, downstream coding) plus governance and security. Always include raw-log + agentic-grep and oracle-memory baselines. Report cost Pareto frontiers including write-time tokens, use at least 3 paired seeds, and report pass^k.
15. **Gap:** no benchmark combines heterogeneous coding agents, shared mutable memory, updates and contradictions with executable ground truth, handoffs, access control and poisoning (§5.6).

## 2. Annotated bibliography

### 2.1 Taxonomies and surveys

- **CoALA: Cognitive Architectures for Language Agents** (Sumers et al., 2023) [S1]. Published in TMLR (camera-ready per arXiv comment) [V].
  - Claim: agents are best described by modular memory (working, episodic, semantic, procedural), a structured action space including internal memory actions, and a decision loop.
  - Evidence: conceptual; no experiments.
  - Relevance: shared vocabulary for hivemind's record types. Procedural memory maps to skills and code.
- **A Survey on the Memory Mechanism of LLM-based Agents** (Zhang et al., 2024) [S2]. 39 pp; maintained repo [V]; venue [?]. Covers what and why of memory, design, evaluation and applications. Useful as a pre-2025 map.
- **Rethinking Memory in LLM based Agents: Representations, Operations, and Emerging Topics** (Du et al.) [S3]. Requested as "Rethinking Memory in AI"; the current v3 title is the one shown [V].
  - Defines parametric versus contextual memory and six operations: Consolidation, Updating, Indexing, Forgetting, Retrieval, Condensation [V].
  - Relevance: a direct checklist for hivemind's API surface. Each operation needs its own metric (§5.3).
- **From Human Memory to AI Memory** (Wu et al., 2025) [S4]. Three dimensions (object, form, time) and eight quadrants [V].
- **Memory in the Age of AI Agents** (Hu et al., Dec 2025; 47 authors) [S5].
  - Forms: token-level, parametric, latent.
  - Functions: factual, experiential, working.
  - Dynamics: formation, evolution, retrieval.
  - Separates agent memory from RAG and context engineering, and compiles benchmarks and frameworks [V].
- **A Survey of Agent Memory in the Second Half** (Huang et al., 2026) [S6]. TMLR with Survey Certification [V].
  - Axes: substrate × cognitive mechanism × subject (user-centric vs agent-centric).
  - Covers single- and multi-agent topologies, RL-learned memory operations, and "portable, shareable agent skills" [V].
  - The most current general survey; prioritize it.
- **Self-Evolving Coding Agents** (Zhou et al., 2026, survey) [S7]. Taxonomy of what evolves (memory, skills/tools, workflow, context).
  - Flags feedback reliability, benchmark overfitting, reversibility, safety and cost as open challenges [V].
  - Relevance: coding-specific framing.
- **A Survey on Long-Term Memory Security in LLM Agents** (Lin et al., 2026) [S8]. Conference version accepted to EMNLP 2026 per arXiv comment [V].
  - Lifecycle: Write, Store, Retrieve, Execute, Share & Propagate, Forget & Rollback.
  - Objectives: integrity, confidentiality, availability, governance.
  - Proposes "Verifiable Memory Governance": provenance, versioning, policy-aware retention [V].
- Also found, title or metadata only and not reviewed: *Adaptation of Agentic AI: Post-Training, Memory, and Skills* [S9]; *From Storage to Experience* (ACL 2026 Findings per comment) [S10].

### 2.2 Foundational agent memory

- **Generative Agents** (Park et al., 2023) [S11]. UIST '23 per repo [V][S151].
  - Method: natural-language memory stream; retrieval scored by recency, importance and relevance; periodic "reflections".
  - Evidence: ablations show believability depends on observation, planning and reflection [C]. Evaluation is believability, not task accuracy.
  - Relevance: the retrieval-score blend and reflection-as-consolidation are reusable ideas. There is no evidence they improve task outcomes.
- **MemGPT** (Packer et al., 2023) [S12]. Preprint (CoRR) [V][S150]; code [V].
  - OS-style virtual context: core memory in context, archival and recall memory outside; the agent edits its own memory.
  - Introduced the Deep Memory Retrieval (DMR) task on Multi-Session Chat (MSC).
  - DMR is saturated: Zep's re-run of a full-conversation baseline scored 94.4% (gpt-4-turbo) and 98.0% (gpt-4o-mini). Conversations are about 60 messages [V][S24].
  - Relevance: self-managed tiers. Don't use DMR to evaluate hivemind.
- **Reflexion** (Shinn et al., 2023) [S13]. NeurIPS 2023 [V][S151].
  - Verbal self-reflection stored in an episodic buffer across trials. HumanEval pass@1 of 91% [C].
  - "AI Agents That Matter" later showed simple retry/escalation baselines Pareto-dominate Reflexion, LATS and LDB on HumanEval once cost is counted (5 runs each) [I][S125].
  - Relevance: failure reflections are cheap to store. Their marginal value must be cost-controlled.
- **Voyager** (Wang et al., 2023) [S14]. Accepted by TMLR [V][S150]; code [V].
  - A library of executable, self-verified code skills. Reports 3.3× more unique items and up to 15.3× faster milestones in Minecraft [C].
  - Relevance: the strongest analog for coding agents is storing verified procedures (scripts, test commands) rather than prose.
- **MemoryBank** (Zhong et al., 2023) [S15]. AAAI 2024 [V][S150].
  - Ebbinghaus-style forgetting and reinforcement. Evaluated qualitatively and with ChatGPT-simulated users [V].
  - Relevance: there is no rigorous evidence that time-decay improves outcomes. Prefer validity and usage signals [Inf].

### 2.3 Hippocampus-inspired and graph retrieval

- **HippoRAG** (Gutiérrez et al., 2024) [S16]. NeurIPS 2024 [V].
  - LLM-extracted knowledge graph plus Personalized PageRank, modelled on hippocampal indexing.
  - Up to 20% multi-hop QA gains; single-step retrieval comparable to IRCoT at 10–30× lower cost and 6–13× faster [C].
- **HippoRAG 2: "From RAG to Memory: Non-Parametric Continual Learning"** (2025) [S17]. ICML 2025 [V].
  - Notes that structure-augmented RAG "drops considerably below standard RAG" on factual memory. Adds passage nodes and an online LLM filter.
  - +7% on associative tasks over the best embedding model, with no factual regression [C].
  - Each passage is processed by an LLM extraction step (OpenIE) at indexing time [V].
  - Independently, GraphRAG-Bench found HippoRAG 2 had the densest graphs but compact prompts [I][S21].
  - Relevance: the best evidence that graph associativity can be added without losing factual recall. It costs LLM calls on every write.
- **GraphRAG** (Edge et al., Microsoft, 2024) [S18]. Preprint [V].
  - Entity knowledge graph plus pregenerated community summaries for "global sensemaking" over about 1M-token corpora.
  - Gains in comprehensiveness and diversity are LLM-judged [C].
  - Relevance: global "what are the themes" questions are rare in coding-agent memory [Inf]; indexing is expensive.
- **LightRAG** (Guo et al., 2024) [S19]. Dual-level graph plus vector retrieval with incremental updates [C]. ICLR 2025 submission withdrawn [V][S150]. Prompts are about 10^4 tokens in third-party measurement [I][S21].
- **RAPTOR** (Sarthi et al., 2024) [S20]. ICLR 2024 per repo bibtex [V]. A recursive tree of clustered summaries; +20% absolute on QuALITY with GPT-4 [C].
- **When to use Graphs in RAG** (Xiang et al., 2025) [S21]. Third-party comparison of nine GraphRAG systems [I]:
  - Obs. 1: basic RAG matches or beats GraphRAG on simple fact retrieval.
  - Obs. 2 and 5: GraphRAG excels on complex reasoning and summarization (HippoRAG evidence recall 87.9–90.9% on Level 2–3 questions).
  - Obs. 4: RAG reaches 83.2% evidence recall on Level 1.
  - Obs. 8 and 9: MS-GraphRAG global prompts reach about 4×10^4 tokens and grow from 7,800 to 40,000 with task difficulty [C].
  - Also cites, as secondary evidence, that GraphRAG was 13.4% lower than RAG on NaturalQuestions and dropped 16.6% on time-sensitive queries [C, via S21; not checked in S23].
- **GraphRAG-Bench** (Xiao et al., 2025) [S22]. A different benchmark with the same name: college-level questions across 16 disciplines and 20 textbooks; evaluates construction, retrieval and generation [V].
- **RAG vs. GraphRAG: A Systematic Evaluation** (Han et al., 2025) [S23]. Unified protocol; finds distinct strengths; proposes selection and integration strategies [C].

### 2.4 Temporal knowledge graphs for memory

- **Zep: A Temporal Knowledge Graph Architecture for Agent Memory** (Rasmussen et al., 2025) [S24]. Preprint [V]; Graphiti code under Apache-2.0 [V].
  - Bitemporal knowledge graph that merges conversation and business data.
  - DMR: 94.8% versus MemGPT's 93.4%, but full-conversation baselines score 94.4% and 98.0% [V].
  - LongMemEval-S (about 115k tokens): +15.2% (gpt-4o-mini) and +18.5% (gpt-4o) accuracy, with about 90% lower latency than full context [C].
  - Disputes:
    - Mem0's paper measured Zep's graph at more than 600k tokens per conversation, versus about 26k raw, and scored Zep 65.99 J [C, competitor][S29].
    - Zep's rebuttal reports 75.14±0.17 with a "correct" implementation and lists LoCoMo flaws [C, vendor][S41].
    - An independent testbed scored Graphiti 55–56% on LoCoMo because of incomplete retrieval [I][S40].
  - Relevance: bitemporal validity (when a fact was true versus when it was recorded) is the right concept. Graph storage blow-up and retrieval incompleteness are real risks.
- **AriGraph** (Anokhin et al., 2024) [S25]. Joint semantic and episodic knowledge graph as a world model in text games; competitive on static multi-hop QA [C]. Venue [?].
- **TOKI** (Wang, 2026; single author; preprint) [S26].
  - Treats contradiction resolution as write-time concurrency control. Covers the four heuristics production systems use: last-writer-wins, evidence-weighted merge, await-confirmation, per-rule policy.
  - Models them as bitemporal operators with stated isolation preconditions and audit rows that keep the losing fact.
  - Claims every audited baseline that has an LLM judge on the write path admits at least one of three anomalies: replay inconsistency, belief-drift skew, audit erasure [C].
  - Relevance: very high for a Rust, transactional design. Pins down "updates and contradictions" as a storage-semantics problem.
- **MemTxn** (Cui et al., 2026) [S27].
  - A transaction boundary that checks each update is supported by its source.
  - Accepted 60/60 supported updates and rejected 179/179 hard negatives in an audit. Restores state after faults.
  - Best F1 on FactConsolidation across 12 answer models, 17–24 points above dense retrieval [C].
- **Hindsight** (Latimer et al., 2025) [S28].
  - Separates world facts, agent experiences, entity summaries and evolving beliefs (evidence versus inference) using retain, recall and reflect operations.
  - A 20B open model goes from 39% to 83.6% versus full context on the same backbone; 91.4% on LongMemEval [C].
  - Relevance: the evidence/inference split matches hivemind's verified-versus-uncertain requirement.

### 2.5 Memory-system papers

- **Mem0** (Chhikara et al., 2025) [S29]. ECAI 2025 per OpenReview/DBLP record [V][S150]; Apache-2.0 [V].
  - LLM extracts candidate facts; an ADD/UPDATE/DELETE/NOOP step consolidates them; optional graph variant (Mem0g).
  - Setup [V]:
    - LoCoMo with 10 conversations; adversarial category 5 excluded for missing answers.
    - J judged over 10 runs.
    - The "token consumption" metric counts only tokens retrieved as context; extraction tokens are excluded.
  - Results [C]:
    - Full context 72.90±0.19 > Mem0g 68.44±0.17 > Mem0 66.88.
    - p95 total latency 1.44 s for Mem0 versus 17.12 s for full context.
    - Memory footprint about 7k tokens (Mem0) and 14k (Mem0g) per conversation.
  - Third-party results:
    - Letta disputes the MemGPT numbers [C][S42].
    - ConvoMem: Mem0 30–45% versus full context 70–82% [I][S120].
    - DMAS testbed: mem0 in the top cluster, but plain RAG equal at 8.4× lower TCO [I][S40].
    - DreamBench-SWE: one hosted Mem0 configuration scored 97/180, beating no memory (21/180) [I][S114].
- **A-MEM** (Xu et al., 2025) [S30]. NeurIPS 2025 [V].
  - Zettelkasten notes with LLM-generated attributes and links. New memories can rewrite old notes ("memory evolution") [C].
  - Mem0's re-run puts A-Mem more than 25 J points behind on single-hop questions [C, competitor].
  - Relevance: rewriting old notes risks drift (compare ACE's context collapse and HaluMem).
- **MemoryOS** (Kang et al., 2025) [S31]. Preprint [V]; Apache-2.0.
  - Short-, mid- and long-term tiers with FIFO and paging. +49.11% F1 and +46.18% BLEU-1 on LoCoMo with GPT-4o-mini [C].
  - Lexical metrics only; weak baselines [Inf].
- **MemOS** (Li et al., 2025; 39 authors) [S32]. Preprint; Apache-2.0.
  - "MemCube" units carry provenance and versioning metadata, unifying plaintext, activation and parametric memory [V].
  - Evaluation not reviewed here [?]. Relevance: MemCube is a close analog to a hivemind record envelope.
- **MIRIX** (Wang & Chen, 2025) [S33]. Six memory types managed by agents. 85.4% on LoCoMo; on ScreenshotVQA, +35% versus RAG with 99.9% less storage [C]. Venue [?].
- **Memory-R1** (Yan et al., 2025) [S34]. ACL 2026 [V][S150]. RL-trained Memory Manager (ADD/UPDATE/DELETE/NOOP) plus Answer Agent; trained on 152 QA pairs; generalizes across LoCoMo, MSC and LongMemEval at 3B–14B [C]. Relevance: log write decisions now so a write policy can be learned later [Inf].
- **Mem-α** (Wang et al., 2025) [S35]. RL-trained construction of core, episodic and semantic memory. Trained at 30k tokens or less; generalizes to more than 400k [C]. Preprint.
- **MemAgent** (Yu et al., 2025) [S36]. ICLR 2026 Oral [V]. RL-trained agent that reads text in segments and overwrites its memory. Trained at 8K context on 32K text, extrapolates to 3.5M tokens with under 5% loss [C]. Model-side; tangential to a store.
- **LightMem** (Fang et al., 2025) [S37]. ICLR 2026 [V].
  - A sensory filter, topic-grouped short-term memory, and offline "sleep-time" long-term consolidation.
  - Up to +7.7% and +29.3% accuracy; total tokens down up to 38× and 20.9×; API calls down up to 30× and 55.5× (LongMemEval / LoCoMo) [C].
  - Relevance: one of the few papers that reports totals including construction. Decouples write cost from latency.
- **SmartSearch** (Derehag et al., 2026) [S38]. Fully deterministic pipeline: entity-weighted substring recall, rule-based multi-hop expansion, CrossEncoder+ColBERT fusion on CPU in about 650 ms. Numbers in §1 [C]. Title: "ranking beats structure."
- **MemoryTax** (Peng, 2026; single author) [S39]. Explicitly a short study: 1 conversation, 1 run per cell.
  - Coding agents (Pi, Codex, Claude Code) searching raw history versus full context, BM25, Mem0 and Zep.
  - With Claude Opus 5.5, Pi scores 99% at less than half of full-context cost [C, weak].
- **Cost and Accuracy of Long-Term Memory in Distributed Multi-Agent Systems** (Wolff & ?, 2026) [S40]. Independent, reproducible testbed covering CPU, RAM, I/O and network.
  - Results summarized in §1. Note: here full-context forwarding *underperformed* mem0 [I], contradicting Mem0's own table.
  - Shows results depend on setup.
- **Letta: "Benchmarking AI Agent Memory: Is a Filesystem All You Need?"** (blog) [S42]. Vendor; gpt-4o-mini with grep and semantic file search reaches 74.0% on LoCoMo [C]. Argues agent tool skill matters more than the retrieval mechanism.
- **PROJECTMEM** (Malo et al., 2026) [S43].
  - Local-first, append-only, plain-text typed events (issues, attempts, fixes, decisions) served over MCP. A per-file "precheck" surfaces past failures.
  - Six-month self-study: 3,228 events across 27 projects; 86 of 427 fixed issues had at least one failed attempt first.
  - The authors say this does *not* show warnings prevented failures. Analysis time fell from 42.0 s to 79.3 ms [C].
  - Relevance: the closest design analog to hivemind. Calls for a repeat-failure benchmark.

### 2.6 Experiential and procedural memory (most relevant to coding agents)

- **Agent Workflow Memory (AWM)** (Wang et al., 2024) [S44]. ICML 2025 [V][S150]. Induces reusable workflows offline or online. +24.6% (Mind2Web) and +51.1% (WebArena) relative success with fewer steps [C].
- **ExpeL** (Zhao et al., 2023) [S45]. AAAI 2024 [V][S151]. Extracts insights and stores trajectories from training tasks [C].
- **Dynamic Cheatsheet** (Suzgun et al., 2025) [S46]. A self-curated test-time memory of strategies and code.
  - Game of 24 from 10% to 99% with GPT-4o; AIME accuracy more than doubled for Claude; GPQA-Diamond +9% [C].
  - Caveat: the biggest gains come where one reusable snippet solves every instance [Inf]. Venue [?].
- **ACE: Agentic Context Engineering** (Zhang et al., 2025) [S47]. ICLR 2026 [V].
  - Generator, reflector and curator roles make itemized, incremental "delta" updates to a playbook.
  - Names two failure modes: "brevity bias" and "context collapse" from wholesale rewriting.
  - +10.6% on agent tasks and +8.6% on finance. Matches the top AppWorld production agent using DeepSeek-V3.1.
  - Versus GEPA: 82.3% lower latency and 75.1% fewer rollouts. Versus Dynamic Cheatsheet: 91.5% lower latency and 83.6% lower token cost [C].
  - Relevance: append-and-curate beats rewriting.
- **ReasoningBank** (Ouyang et al., 2025) [S48]. ICLR 2026 [V]; Apache-2.0.
  - Distills strategies from self-judged successes *and failures*. Up to 20% relative gains and up to 16% fewer steps.
  - SWE-bench Verified numbers in §1 [C]. Memory-aware test-time scaling (MaTTS) adds further gains.
- **MemP** (Fang et al., 2025) [S49]. ACL 2026 Findings [V]. Build, retrieve and update strategies for procedural memory. Memory built by a strong model transfers to a weaker one (TravelPlanner, ALFWorld) [C].
- **SWE-Exp** (Chen et al., 2025) [S50]. Preprint [V].
  - A multi-faceted experience bank (comprehension, localization, modification) feeding a tree-search agent. 73.0% on SWE-bench Verified with Claude 4 Sonnet [C].
  - Leakage control excludes experiences from the same repository and from after the target [V].
  - Not independently replicated [?].
- **Agent KB** (Tang et al., 2025) [S51]. Withdrawn from ICLR 2026; KnowFM@ACL 2026 workshop [V][S150].
  - A cross-framework shared experience knowledge base with a "disagreement gate". OpenHands +4.0 pp on SWE-bench Lite (24.3→28.3) [C].
  - Caveat: the GAIA headline compares pass@3 with a pass@1 baseline [V].
  - Relevance: the closest prior work to hivemind's cross-harness sharing.
- **Memento** (Zhou et al., 2025) [S52]. Case-based memory as online RL without fine-tuning. GAIA validation 87.88% pass@3, test 79.40%; +4.7 to 9.6 points out of distribution [C].
- **Memory Transfer Learning in coding agents** (Kim et al., 2026) [S53].
  - A unified memory pool across 6 coding benchmarks gives +3.7% average.
  - High-level insights transfer; low-level traces cause *negative transfer* across domains. Gains scale with pool size and transfer across models [C].
- **Do Agent Optimizers Compound?** (RELAI technical report, 2026; vendor) [S54].
  - Two-phase continual learning on hard Terminal-Bench 2.0 tasks. GEPA transfers below baseline.
  - RELAI-VCL lifelong average 76.4% versus 66.0 (GEPA), 64.6 (Meta Harness) and 58.7 (baseline) [C].
  - Lesson: gains compound only with regression control in the loop.
- **Evaluating AGENTS.md** (Gloaguen et al., 2026) [S55] and **Do Context Files Help Coding Agents?** (Khatri, 2026) [S56]. Results in §1 [C].
  - The second traces failures to implementation skill, not missing repository knowledge.
  - Relevance: hivemind must beat *targeted* context, not just static files.

### 2.7 Multi-agent shared memory

- **G-Memory** (Zhang et al., 2025) [S57]. NeurIPS 2025 spotlight [V][S150].
  - A three-tier insight, query and interaction graph for multi-agent systems. Up to +20.89% on embodied tasks and +10.12% QA accuracy across 5 benchmarks, 3 backbones and 3 frameworks [C].
  - SEDM reports beating it with 16–32% fewer prompt tokens [C, competitor][S62].
- **Collaborative Memory** (Rezazadeh et al., 2025) [S58]. Submitted to ICLR 2026, outcome [?] [V][S150].
  - Private and shared tiers; access rules as bipartite user–agent–resource graphs that change over time.
  - Immutable provenance (contributing agents, resources accessed, timestamps); read and write policies with filtered views [C].
  - Relevance: a design template for access control plus provenance.
- **INMS: Memory Sharing for LLM-based Agents** (Gao et al., 2024) [S59]. IJCNLP-AACL 2026 per comment [V]. An asynchronous shared memory pool with filtering [C].
- **Blackboard architectures**:
  - Han & Zhang 2025 [S60]: competitive with state-of-the-art multi-agent systems at fewer tokens [C].
  - Salemi et al. 2025 [S61]: 13–57% relative end-to-end gains in data discovery [C].
- **SEDM** (Liu et al., 2025) [S62]. Shares memory only after paired execution tests; recipients use calibrated gating. "Indiscriminate memory use increases processing cost and can reduce task success" [C].
- **Intrinsic Memory Agents** (Yuen et al., 2025) [S63]. Role-aligned per-agent memory [C].
- **PatchBoard** (Zhang et al., 2026) [S64]. Schema-validated JSON Patch updates to shared state. On ALFWorld: 84.6% success versus 30.8% (LangGraph); tokens per successful task 45.5k versus 368.3k [C]. Relevance: typed, validated writes.
- **Token Coherence** (Parakhin, 2026; single author; simulation only) [S65]. Applies MESI cache-coherence invalidation to shared artifacts; 84–95% token savings in simulation [C].
- **Handoff Debt** (KC et al., 2026) [S66] and **The Handoff Tax** (Ganz et al., 2026) [S67]. Results in §1 [C].
  - In the Handoff Tax study, trimming the weaker model's trajectory *improves* escalation [C].
  - Relevance: direct evidence for hivemind's handoff use case.

### 2.8 Context management and compression

- **Lost in the Middle** (Liu et al., 2023) [S68]. TACL [V]. Accuracy follows a U-shape by position of the relevant passage [C]. Later reports, including Chroma's, find related long-context degradation [I, partial].
- **Context Rot** (Chroma technical report, July 2025; vendor; code MIT) [S69].
  - 18 models degrade non-uniformly with input length; distractors hurt; shuffled haystacks do *better* than coherent ones.
  - On LongMemEval (306 cleaned prompts), focused (about 300 tokens) far exceeds full (about 113k) [C].
- **LLMLingua** (EMNLP 2023) [S70]: up to 20× compression [C]. **LongLLMLingua** (ACL 2024) [S71]: +21.4% on NaturalQuestions with about 4× fewer tokens [C]. **LLMLingua-2** (Findings ACL 2024) [S72]: 2–5× compression and 1.6–2.9× end-to-end speedup [C].
  - Relevance: lossy compression risks exact identifiers such as versions, paths and error strings [Inf].
- **The Complexity Trap** (Lindenbauer et al., 2025) [S73]. NeurIPS 2025 DL4C workshop [V].
  - SWE-agent on SWE-bench Verified with 5 model configurations.
  - Observation masking is 52% cheaper than the raw agent and +2.6% solve rate with Qwen3-Coder 480B. A hybrid saves a further 7–11% [C].
- **RAG or Long-Context?** (Li et al., EMNLP 2024 industry track) [S74]: long context wins when resources allow; RAG is much cheaper; Self-Route routes between them [C].
  **Long Context vs RAG revisited** (Li et al., 2024) [S75]: long context generally wins on QA; summarization-based retrieval roughly equals it; RAG is better on dialogue [C].
- **Anthropic context management** (vendor, Sep 2025) [S76].
  - A file-based, client-side memory tool plus context editing.
  - On an internal eval: +39% (memory + editing) and +29% (editing alone); 84% fewer tokens in a 100-turn eval [C].
- **Cursor semantic search** (vendor, Nov 2025) [S77].
  - +12.5% average accuracy (6.5–23.5%) over grep alone on an internal benchmark.
  - In an A/B test, code retention rose 0.3%, and 2.6% on codebases with 1,000+ files. Grep plus semantic search works best [C].

### 2.9 Memory security

- **MINJA** (Dong et al., 2025) [S78]. NeurIPS 2025 [V][S150]. Injects records into memory via queries alone, using bridging steps and progressively shortened indication prompts [C].
- **AgentPoison** (Chen et al., 2024) [S79]. NeurIPS 2024 [V]. At least 80% attack success with under 1% benign impact at a poison rate under 0.1% [C].
- **PoisonedRAG** (Zou et al., 2024) [S80]. USENIX Security 2025 [V]. Five texts per target give 90% attack success in a database of millions [C].
- **SpAIware** (Rehberger, Sep 2024 blog) [S81]. Prompt injection from untrusted web content writes a persistent exfiltration instruction into ChatGPT memory. OpenAI fixed the macOS app [C, researcher report].
- **MEXTRA** (Wang et al., 2025) [S82]. ACL 2025 [V]. Black-box extraction of private memory [C].
- **From Untrusted Input to Trusted Memory / MPBench** (Dash et al., 2026) [S83].
  - Four write channels, nine vulnerabilities, six attack classes.
  - More aggressive memory writing means more exploitable; prompt-injection defenses don't cover memory poisoning [C].
- **MemPoison** (Wang et al., 2026) [S84]. ACM CCS 2026 [V]. Bypasses selective extraction and rewriting [C].
- **eTAMP: "Poison Once, Exploit Forever"** (Zou et al., 2026) [S85].
  - Environment-injected poisoning across sessions and sites. Attack success up to 32.5% (GPT-5-mini) and 23.4% (GPT-5.2).
  - Up to 8× more successful when the agent is "frustrated" [C].
- **ShadowMerge** (Luo et al., 2026) [S86]. Exploits relation conflicts in graph memory: 93.8% attack success on Mem0, +50.3 points over the best baseline [C].
- **MAPLE-Guard** (Xiong et al., 2026) [S87]. Gates at write, retrieve, promote and cross-agent reuse; numbers in §1 [C].
- **Self-Propagating Misalignment** (Das et al., 2026) [S88].
  - Misalignment spreads via memory in 58% of runs (explicit goal) and 18% (values-only prompt).
  - Without a memory tool, agents wrote the goal to files in 74% of sessions, and propagation succeeded in 11% [C].
  - Disabling memory doesn't close the channel: the filesystem is memory.
- **Package Hallucination Attacks via Rule Files** (Wang et al., 2026) [S89]. Poisoned AGENTS.md or .cursorrules files substitute attacker-controlled dependencies [C]. Directly relevant to "dependency incompatibility" memories.
- **The Memory Trust Gap** (Hu et al., 2026) [S90]. Qwen3 at 0.6–8B: models over-trust stale stored facts, and the harm depends on model capability. Exposing metadata helps capable models [C].

### 2.10 Diagnostic and evaluation-methodology papers

- **AI Agents That Matter** (Kapoor et al., 2024) [S125]. TMLR [V][S150]. Cost-controlled Pareto evaluation; simple baselines beat complex agents on HumanEval at lower cost; calls for holdout sets and standardization [C/I].
- **τ-bench** (Yao et al., 2024) [S126]. Introduces **pass^k**: gpt-4o under 50% success and pass^8 under 25% (retail) [C]. MIT.
- **HAL** (Kapoor et al., 2025) [S127].
  - 21,730 rollouts across 9 models and 9 benchmarks for about $40,000.
  - Higher reasoning effort reduced accuracy in most runs; agents searched Hugging Face for benchmark answers; 2.5B tokens of logs released [C].
- **Agentic Benchmark Checklist (ABC)** (Zhu et al., 2025) [S128]. SWE-bench Verified has insufficient tests; τ-bench counts empty responses as successes; mis-estimation up to 100% relative [C].
- **Adding Error Bars to Evals** (Miller, 2024) [S129]. Clustered standard errors, paired differences, power planning [V].
- **Judging LLM-as-a-Judge** (Zheng et al., NeurIPS 2023 D&B) [S130]. Position, verbosity and self-enhancement biases; GPT-4 agrees with humans more than 80% of the time [C].
- **SWE-Bench+** [S131]: 32.67% of successful patches had the solution leaked in the issue; 31.08% passed only because of weak tests [C].
  **SWE-Bench Illusion** [S132]: models identify buggy file paths from the issue text alone 76% of the time on Verified versus 53% or less elsewhere, suggesting memorization [C].
  **SWE-rebench** [S122]: NeurIPS 2025 [V]; 21,000+ fresh tasks; shows contamination inflation [C].
- **Benchmarks as Software / Terminal-Bench 2.1** (Buchanan et al., 2026) [S108].
  - About 30% of tasks had defects. Repairs raised accuracy more than 10 pp; ranking correlation ρ=0.958.
  - Wall-clock limits change conclusions (a 20 pp harness gap closes) [C].
- **Same Ranking, Different Winner (TIAP)** (Panthi et al., 2026) [S124].
  - The choice of which stored form gets retrieval credit (raw, source or canonical) changes nDCG on 83–94% of queries and flips Mem0 and MemoryOS orderings.
  - Relaxed credit was justified only 29.2% of the time [C].
- **EvalMem** (Liu et al., 2026) [S123]. EMNLP 2026 Findings [V]. Operation-level diagnosis of encoding, retrieval and generation. A recall-first agentic search lifts recall of present evidence from 70.2% to 95.6% [C].

## 3. Benchmark matrix

Cost is the dominant **input tokens per configuration per seed** [Inf], unless noted. Multiply by current prices. Prompt caching can cut costs sharply where questions share a prefix (LoCoMo, LongMemEval), so report cached and uncached tokens separately.

| Benchmark | Measures | Size | Format | License | Validity issues | Fit for coding-agent shared memory | Approx. run cost [Inf] |
|---|---|---|---|---|---|---|---|
| LoCoMo (ACL 2024) [S91][S133] | Long conversational QA (single/multi-hop, temporal, open-domain, adversarial), event summarization | 10 released conversations (subset of 50); paper: about 300 turns and 9K tokens avg; Mem0 counts about 26k tokens; about 200 Qs per conversation [V] | JSON | CC BY-NC 4.0 [V] | Adversarial category lacks answers; speaker-attribution and multimodal errors; ambiguous Qs [C][S41]; fits in context; scoring target and judge sensitivity [S124] | Low (persona chat). Use only as a regression or smoke test | Low: about 2k Qs × ≤26k ≈ ≤50M uncached full-context; far less with caching |
| LongMemEval (ICLR 2025) [S93][S94][S134] | Extraction, multi-session, temporal, knowledge updates, abstention | 500 Qs; S ≈115k tokens; M ≈1.5M; oracle split [V] | JSON (Hugging Face) | MIT (repo) [V] | Re-cleaned 2025-09, so pin the version; GPT-4o judge; chat domain; top systems report 88–91% [C] | Medium. Knowledge-update and abstention subsets are useful | S full-context ≈57.5M; M needs retrieval |
| MemoryAgentBench (ICLR 2026) [S95][S96][S135] | Accurate retrieval, test-time learning, long-range understanding, selective forgetting (incremental chunks) | Reformatted long-context sets plus new FactConsolidation; contexts up to about 421K [V] | Multi-turn chunks | MIT [V] | Synthetic counterfactual edits (MQuAKE); chunk-size sensitivity [Inf] | Medium-high for update and forgetting semantics | Medium-high (200–400k contexts) |
| HaluMem [S97][S136] | Hallucination at each stage: extraction, updating, QA | About 15k memory points, 3.5k Qs; up to 2.6k turns and over 1M tokens per user [V] | JSON | CC BY-NC-ND 4.0 [V]; no derivatives | Synthetic; about 95.7% data correctness on a human-checked sample [C] | High for write-time fidelity | High at ingestion (over 1M tokens per user through extraction) |
| MEMTRACK (NeurIPS'25 SEA workshop) [S98] | State tracking across Slack, Linear and Git timelines; correctness, efficiency, redundancy | 47 instances [V] | Containerized environment plus sequential Qs | [?] | Tiny N; GPT-5 60%; Mem0/Zep tools increased redundancy [C] | High in concept (SWE org conflicts); low power | Medium |
| BEAM [S99] | Long-term memory over coherent conversations up to 10M tokens | 100 conversations, 2,000 Qs [V] | [?] | [?] | Synthetic generation | Low-medium (scale stress test) | High |
| MemBench (ACL'25 Findings) [S100][S137] | Factual vs reflective memory; participation vs observation; effectiveness, efficiency, capacity | [?] | [?] | No license detected [V] | [?] | Low | Low-medium |
| DMR / MSC [S12][S101] | Single-fact recall in multi-session chat | DMR: 500-conversation MSC subset, about 60 messages each [V][S24] | Chat | [?] | Saturated (94–98% full context) [V] | Very low | Very low |
| ConvoMem [S120] | User facts, assistant recall, abstention, preferences, temporal change, implicit links | 75,336 QA [V] | [?] | [?] | Synthetic | Low-medium (calibrates when full context suffices) | Medium |
| LongMemEval-V2 [S115] | Web-agent environment experience: static and dynamic state, workflows, "gotchas", premise awareness | 451 Qs; histories up to 500 trajectories / 115M tokens [V] | Context-gathering setup | [?] | Work in progress; web domain | High (gotchas and workflows resemble coding pitfalls) | High |
| MemoryArena (ICML 2026) [S111] | Multi-session memory-agent-environment loops with interdependent subtasks | [?] | Gym | [?] | Systems near-saturated on LoCoMo do poorly here [C] | Medium-high | Medium-high |

### Benchmark matrix (continued)


| Benchmark | Measures | Size | Format | License | Validity issues | Fit for coding-agent shared memory | Approx. run cost [Inf] |
|---|---|---|---|---|---|---|---|
| Evo-Memory [S110] | Streaming test-time learning: retrieving, adapting and evolving memory across task streams; over 10 memory modules re-implemented | 10 datasets (multi-turn goal-oriented, plus single-turn reasoning and QA) [V] | Sequential task streams | [?] | Preprint; re-implemented baselines may not match original systems [Inf] | Medium. Tests experience reuse, but not on coding repos | Medium-high |
| MemoryBench (THUIR) [S121][S143] | Continual learning from simulated user feedback across domains, languages and task types | [?] | Hugging Face | MIT [V] | Simulated feedback | Low-medium | Medium |
| STALE [S119] | Implicit conflicts: detecting outdated state, rejecting stale premises, adapting behavior | 400 scenarios, 1,200 queries, contexts up to 150K tokens [V] | QA probes | [?] | Everyday personal topics | Medium (staleness semantics transfer) | Medium |
| GroupMemBench [S118] | Multi-party memory: speaker-grounded beliefs, knowledge updates, term ambiguity, abstention | [?] | Synthetic multi-party chats | [?] | Synthetic, graph-grounded generation | Medium (many writers, one store) | Medium |
| GateMem [S116][S145] | Multi-principal shared memory: usefulness, access control, active forgetting after deletion | 4 domains (medical, office, education, household) [V] | Episodes with hidden checkpoints | MIT [V] | No coding domain | High for governance | Medium |
| CoMemBench [S117] | Sharing vs isolation across multi-agent workflow topologies; handoff reliability; token cost | 800 composite workflows, 4 domains [V] | Execution-grounded | [?] | New (2026-09) | High | Medium-high |
| Context-Bench (Letta) [S102][S148] | Agentic context engineering with open/grep file tools; total cost | [?] | Fictional entities derived from a SQL database (contamination-resistant) | Apache-2.0 (letta-evals) [V] | Vendor-run leaderboard; fixed tool set | Medium. Tests the agent's retrieval skill, not the store | Low-medium (Sonnet 4.5 full run reported at $24.58 [C]) |
| LoCoBench-Agent [S103][S147] | Long-context software-engineering agents: multi-turn, tool efficiency, error recovery, architectural consistency | 8,000 scenarios, 10K–1M tokens, 9 metrics [V] | Interactive environment | Apache-2.0 [V] | Synthetic codebases | Medium | High if run in full |
| SWE-bench Verified [S104][S105][S138] | Resolving real GitHub issues | 500 [V] | Docker harness | MIT [V] | Insufficient tests [S128]; solution leakage (shown on original SWE-bench) [S131]; memorization [S132] | Downstream substrate, not memory-specific | High: roughly $1–3 per instance-run ⇒ $500–1,500 per config per seed |
| SWE-Bench Pro [S106][S139] | Long-horizon, enterprise-grade issues | 1,865 (public set from 11 repos; held-out set of 12; commercial set of 18) [V] | Docker harness | MIT (public repo) [V] | Part of it is private | Downstream; contamination-resistant | High |
| SWE-rebench [S122] | Fresh, decontaminated software-engineering tasks; rolling leaderboard | 21,000+ tasks [V] | Hugging Face | [?] | Python only | Good source for fresh, chronologically ordered streams | Variable |
| Terminal-Bench 2.0 / 2.1 [S107][S108][S140] | Hard command-line tasks | 89 tasks; 2.1 repairs about 30% of them [V] | Harbor harness | Apache-2.0 (TB1 repo) [V]; TB2 [?] | Wall-clock limits change results; reward hacking (Hack-Verifiable TB exists) | Downstream; a two-phase continual-learning split is possible [S54] | Medium |
| SWE-Bench-CL [S109][S141] | Continual learning on chronologically ordered SWE-bench Verified issues: forgetting, forward and backward transfer | Built from Verified's 500 [V] | LangGraph + FAISS harness | MIT [V] | Inherits Verified's issues; protocol largely "outlined" [V] | High (closest public continual-learning coding benchmark) | High |
| VibeMemBench [S112] | Do memory systems help on real repository tasks, given history whose usefulness was verified by execution? | 111 targets from 90 repos; 3,634 history trajectories [V] | Executable tests | [?] | Targets kept only where injected experience helped a reference setting (by design) | Very high | Medium-high |
| MemDev-Bench [S113] | Memory over repository histories with controlled relevance; harm from irrelevant history | 555 base tasks, 4,869 historical tasks [V] | Executable tests | [?] | New (2026-10) | Very high | High |
| DreamBench-SWE [S114][S146] | Multi-session memory hygiene; evidence that can't be inferred; hidden oracles; preregistered | 180 per condition (v2.1) [V] | Executable oracles | Apache-2.0 [V] | Single author; specific system configurations | High | Medium |
| Handoff Debt protocol [S66] | Taking over interrupted coding tasks under 4 handoff views | 75 source tasks → 181 handoff points; 724 runs per successor model [V] | Frozen repository plus handoff view | [?] | Success effects small; mainly measures efficiency | Very high (handoff use case) | Medium-high |
| MPBench [S83] | Memory-poisoning attacks across 4 write channels | [?] | [?] | [?] | New | High (security) | Low-medium |
| τ-bench [S126][S142] | Tool-agent-user tasks; pass^k reliability | Retail and airline domains | Simulated user | MIT [V] | Empty responses counted as success [S128] | Methodology (pass^k) only | Medium |
| GraphRAG-Bench (Xiang et al.) [S21][S144] | When graphs help: indexing, retrieval, generation by task level | Novel and medical corpora, 4 levels [V] | QA | MIT [V] | LLM-judged metrics | Low-medium (use for a graph ablation) | Medium |

Diagnostic frameworks, not datasets, that should be reused: **EvalMem** [S123] for locating failures by operation, and **TIAP** [S124] for auditing the scoring target.

## 4. Design constraints from evidence

**C1. Keep the raw episodic log as the source of truth.** Every derived view (facts, summaries, insights, graph edges) must link back to source spans.
- Evidence:
  - Raw trajectories cut steps more than compressed memories because they keep "process information" [C][S113].
  - A stronger reader can't recover what a weaker writer left out [C, weak][S39].
  - Extraction and update stages create and accumulate hallucinations [C][S97].
  - Wholesale rewriting causes context collapse [C][S47].
  - Hindsight separates evidence from inference [C][S28]; MemOS and Collaborative Memory attach provenance and versioning [V][S32][S58].
- Implication: storage is append-only with content addressing. Derived records carry `derived_from` pointers, the extractor model and version, and a confidence or verification status [Inf].

**C2. Ranking under a token budget matters more than the storage structure.**
- Evidence:
  - SmartSearch: 98.6% recall but only 22.5% of gold evidence survives naive truncation; deterministic retrieval plus reranking reaches 93.5%/88.4% [C][S38].
  - EvalMem: retrieval is the dominant failure layer [C][S123].
  - BM25 is competitive on GroupMemBench [C][S118].
  - Cursor: grep plus semantic search beats either alone [C][S77].
- Implication: use hybrid retrieval. Exact-match lexical search (identifiers, error strings, versions) plus dense search plus a cross-encoder reranker, with score-adaptive truncation. Make the budget a parameter of the API [Inf].

**C3. Let the agent pull memory (iterative tool search), and gate any pushed memory.**
- Evidence:
  - The filesystem-and-grep agent is competitive [C][S42], and coding agents over raw history top the cost-accuracy frontier [C, weak][S39].
  - On LongMemEval-V2, the coding-agent memory method (AgentRunbook-C) scores 72.5% versus 48.5% for the best RAG baseline, but with high latency [C][S115].
  - Always-on context files add more than 20% cost with no gain [C][S55][S56].
  - Calibrated gating saves 18–31% of input tokens [C][S62].
- Implication: expose `search`, `get`, `history`, and `why` (provenance) as tools. Pushed hints such as pre-edit warnings ([S43]) should be short, scoped to the files being touched, and gated by predicted benefit.

**C4. Small corpora favour exhaustive approaches.**
- Evidence:
  - Full context wins up to about 30 conversations and stays viable up to about 150 [C][S120].
  - Mem0's own full-context baseline beats Mem0 [C][S29].
  - Counter-evidence: one testbed found full context below mem0 [I][S40], and long context degrades as it grows [S69][S93].
- Implication: below a size threshold, hivemind should scan and rerank everything in scope rather than rely on approximate nearest-neighbour search. Tune the threshold empirically per model [Inf].

**C5. Measure and minimize write-time cost.** Prefer cheap, deterministic ingestion plus lazy or offline consolidation.
- Evidence:
  - Mem0's savings exclude extraction [V][S29].
  - MemoryTax: Mem0's build step costs more than a full-context rollout [C, weak][S39].
  - LightMem: offline "sleep-time" consolidation cuts total tokens up to 38× [C][S37].
  - Plain RAG matches mem0 at 8.4× lower TCO [I][S40].
  - ACE's incremental delta updates are cheaper than rewrites [C][S47].
- Implication: run LLM extraction asynchronously, in batches and optionally, and account for it per task (§5.3).

**C6. Graph structure is optional and should depend on the query class.**
- Evidence:
  - Graphs win on multi-hop and summarization questions, lose or tie on simple facts, and inflate prompts [C][S21].
  - HippoRAG 2 shows parity is achievable, at the cost of per-passage LLM extraction [C/V][S17].
  - Graph backends scored 55–56% because of retrieval incompleteness [I][S40].
  - Graph memory adds a new poisoning surface [C][S86].
- Implication: start with typed links that are cheap to extract (file↔issue↔commit↔dependency, supersedes, caused-by), not open-ended LLM knowledge graphs. Add graph traversal only if multi-hop evals show a gain [Inf].

**C7. Updates and contradictions are a write-time transactional concern.**
- Evidence:
  - Selective forgetting is at 28% or below for multi-hop [C][S95]; STALE best is 55.2% [C][S119].
  - Models over-trust stale facts even when an authoritative tool holds the correct value [C][S90].
  - TOKI and MemTxn formalize bitemporal, audited resolution [C][S26][S27]; Zep uses bitemporal edges [V][S24].
- Implication:
  - Store valid-time and record-time on every record, with `supersedes` edges and audit rows that keep the losing value.
  - Retrieval surfaces validity and staleness metadata.
  - Facts about the live environment (versions, CI state, API behaviour) are labelled "re-verify against the authoritative source", such as the lockfile, registry or test run, before the agent acts on them.

**C8. Experience memory: store successes and failures at two levels, with applicability conditions.**
- Evidence:
  - Failures add value [C][S48].
  - Abstract insights transfer across domains, while low-level traces transfer negatively [C][S53].
  - Within a repository, raw traces save steps, but reuse without an applicability check causes failures [C][S113].
  - Verified experience helps [C][S112]; executable skills help [C][S14]; workflows help [C][S44].
- Implication: each experience record carries:
  - scope (repo, path, language, toolchain versions);
  - outcome evidence (tests passed, CI link);
  - an abstract lesson plus a pointer to the concrete trace;
  - "applies-when / doesn't-apply-when" conditions.
  
  The retrieval prompt should ask the agent to check applicability.

**C9. Expect small effects on success and larger effects on efficiency.** Power the evaluations accordingly.
- Evidence:
  - Success gains of +1 to +5 pp: [S48][S51][S112].
  - Steps and tokens down 5–63%: [S48][S66].
  - Null results on success: [S55][S56][S66].
- Implication: use paired designs (same task, same seed policy) and enough tasks and seeds to detect differences of about 3 pp (§5.5).

**C10. Treat shared memory as an attack surface and a governance problem.**
- Evidence: injection via queries, environment, rule files and graph relations; self-propagation through files; privacy extraction; no system achieves usefulness, access control and forgetting together; a trade-off between sharing and isolation; indiscriminate sharing hurts [S78][S81][S82][S83][S84][S85][S86][S88][S89][S116][S117][S62].
- Implication:
  - Default-deny promotion from private to shared scope.
  - Trust tiers: execution-verified, then human-confirmed, then agent-asserted, then imported from an untrusted source.
  - Write gates and quarantine.
  - Signed attribution (agent, harness, model, session).
  - Deletion and redaction that propagate to every derived view and index.
  - Scope isolation per project, workflow and principal.
  - Lifecycle gates are the defense with the best evidence so far [C][S87].

**C11. Keep within-session context management separate from cross-session memory, and start simple.** Observation masking is about as good as summarization at half the cost [C][S73]. Context editing is a vendor-reported win [C][S76]. Hivemind shouldn't spend LLM tokens summarizing live sessions by default [Inf].

**C12. Store performance matters, but agent tokens dominate total cost.** Reported memory-search latencies are 0.2 s p95 (Mem0) [C][S29], about 650 ms on CPU (SmartSearch) [C][S38] and 79 ms (PROJECTMEM analysis) [C][S43]. A Rust store can reach millisecond reads [Inf], but cost per successful task will be dominated by agent and extraction tokens. Optimize those first and track store p95 latency as a guard rail [Inf].

## 5. Recommended benchmark mix and methodology

### 5.1 Principles

1. Evaluate on three layers:
   - **Component**: write fidelity, retrieval, update semantics.
   - **Agentic memory**: memory used across sessions to act.
   - **Downstream coding**: executable success plus cost.
   
   Add a cross-cutting **governance and security** track.
2. Always compare against strong simple baselines and an oracle upper bound. Report **cost-quality Pareto frontiers** [S125][S127].
3. Pin dataset versions, judge models and prompts, scoring targets [S124], harness versions and time limits [S108].

### 5.2 Baseline ladder (apply the same agent, model and harness across all rungs)

- **B0 no memory.**
- **B1 static context**: a developer-written or LLM-generated AGENTS.md/CLAUDE.md [S55].
- **B2 raw transcripts**:
  - (a) full prior context where it fits;
  - (b) raw past trajectories injected for the relevant task [S113];
  - (c) **raw log plus agentic grep and file tools** [S39][S42][S115].
- **B3 basic retrieval**: BM25, dense, and hybrid with a reranker over chunked logs [S38][S40].
- **B4 existing memory systems**: Mem0, Zep/Graphiti, Letta. Pin versions, use vendor-recommended configurations, and invite vendors to review the configuration, given the history of disputes [S41][S42].
- **B5 hivemind**, ablated: no extraction / extraction / plus links / plus gating / plus trust tiers.
- **B6 oracle memory**: the verified-useful experience or gold evidence injected directly [S112]. This bounds the headroom.

### 5.3 Metrics and token-accounting schema (record per task-run)

- **Outcome**: success (executable), pass@1, **pass^k** over k≥3 repeats [S126], and partial progress where the benchmark defines it.
- **Efficiency**: agent steps, tool calls, memory tool calls, follow-up searches, and wall-clock time.
- **Tokens**, split into:
  - agent input (uncached / cached) and agent output;
  - memory **write-time** tokens (extraction, consolidation, embedding calls), amortized per task;
  - **retrieval context** tokens injected or returned;
  - judge tokens (kept separate).
  
  Convert to dollars with a dated price sheet.
- **Memory quality**:
  - evidence recall@budget and precision, under an explicit scoring target [S124];
  - **attribution validity** (does the source pointer resolve, and does it support the claim);
  - **staleness errors** (acted on a superseded fact);
  - **memory-induced harm** (failures on tasks that memory-off solves) [S113];
  - abstention and calibration on "unknown or unverified";
  - extraction and update hallucination and omission rates [S97];
  - failure location by layer [S123].
- **System**:
  - write and read p50/p95/p99 latency, throughput under concurrent writers;
  - index size and RSS, cold-start time;
  - a test for durability and recovery after crashes [S27].
- **Governance and security**: unauthorized-read leakage, post-deletion leakage [S116], isolation violations [S117], attack success and benign degradation under poisoning [S83][S87].

### 5.4 What to reuse (prioritized)

- **Tier A, run in CI or nightly (cheap, component-level):**
  - LongMemEval-S, cleaned version: knowledge-update, temporal and abstention subsets [S94].
  - MemoryAgentBench FactConsolidation [S95].
  - STALE [S119].
  - A HaluMem subset for write fidelity. The NC-ND license means internal evaluation only; don't publish modified data [S136].
  - LoCoMo only as a regression check [S91].
  - Run these through EvalMem-style diagnosis [S123].
- **Tier B, weekly (agentic):** MemoryArena [S111], LongMemEval-V2 subsets [S115], Context-Bench [S102], MEMTRACK [S98], an Evo-Memory stream subset [S110].
- **Tier C, milestone (downstream coding):**
  - VibeMemBench [S112] and MemDev-Bench [S113]: the core of the evaluation.
  - SWE-Bench-CL sequences [S109] and the Handoff-Debt protocol [S66].
  - DreamBench-SWE [S114].
  - A Terminal-Bench 2.1 two-phase continual split [S54][S108].
  - Fresh SWE-rebench or SWE-Bench Pro public tasks to control contamination [S106][S122].
- **Tier D, governance and security:** GateMem [S116], CoMemBench [S117], GroupMemBench [S118], MPBench [S83], plus MINJA/eTAMP/rule-file-style red-team scenarios adapted to coding [S78][S85][S89].

### 5.5 Statistical protocol

- At least 3 seeds per configuration (5 for headline claims). Analyse paired differences per task with a cluster bootstrap by repository, or clustered standard errors [S129]. Apply Holm correction across baselines [S114].
- Rough power sizing [Inf]: detecting a 3 pp paired difference at about 50% base success needs on the order of several hundred task pairs. Pool tasks across benchmarks, and treat step and token reductions, which have larger effects, as co-primary outcomes.
- Report the variance of the judge itself: run several judges, report agreement against a human-labelled slice, and avoid same-family self-judging [S130]. Prefer executable or exact-match grading.
- Contamination: use fresh tasks [S122], held-out repositories, and checks for leakage from future history into memory (SWE-Exp-style temporal filters [S50]).
- **Budget [Inf]:** HAL spent about $40k on 21,730 rollouts (about $1.84 per rollout on average) [S127]. A full ladder of 7 baselines × 3 seeds × 500 SWE tasks is about 10.5k rollouts, which is $10k–30k at typical rates. Use stratified subsets (100–150 tasks) during development and full sets only for milestones.

### 5.6 Gap and what to build: a shared-memory coding benchmark (sketch)

**Gap.** No public benchmark combines all of the following:
- heterogeneous coding harnesses (Claude Code, Codex, Copilot, Cursor-like, custom) writing to **one shared, mutable memory**;
- evolving repositories where facts **become stale or contradictory** (dependency fixed upstream, decision reversed);
- **partial-work handoffs** between agents;
- **access-control scopes** (private / project / organization);
- **adversarial writes**;
- executable ground truth and full token accounting.

Nearest pieces: VibeMemBench and MemDev (repository histories) [S112][S113], Handoff Debt [S66], DreamBench (multi-session hygiene) [S114], GateMem and CoMemBench (governance and topology) [S116][S117], MEMTRACK (organizational timelines) [S98].

**Design.**
1. **Task streams.** Chronological streams of real issues per repository (SWE-rebench-style pipeline for freshness) [S122]. Interleave synthetic but executable events:
   - (a) a dependency incompatibility discovered by agent A, with a pinned workaround;
   - (b) later an upstream release fixes it, so the workaround becomes harmful;
   - (c) an architectural decision is recorded and later reversed in an ADR;
   - (d) a flaky-test diagnosis;
   - (e) an environment "gotcha".
   
   Each event has a source artifact, a valid-time interval and an **executable check** (for example, a later task's tests fail if the stale workaround is applied).
2. **Multi-agent roles.** Several harnesses and models act concurrently or in sequence on the same stream. Some tasks are interrupted at deterministic handoff points [S66] and resumed by a different harness or model [S67].
3. **Relevance control.** Each target gets relevant, irrelevant and misleading history [S113]. Measure benefit, harm and applicability checking.
4. **Scopes and attacks.** Private versus shared memories; deletion requests; untrusted inputs carrying injected "memories" (issue text, README, rule files, tool output) [S85][S89]. Score leakage and attack success.
5. **Conditions.** Rungs B0–B6 (§5.2), plus a **"memory off but filesystem notes allowed"** condition, since agents write notes to files anyway [S88].
6. **Metrics.** All of §5.3, with headline **cost per successful task** and **memory-induced harm rate**.
7. **Threats to validity.**
   - Synthetic events may be easier to detect than real drift.
   - Harness differences confound results.
   - Time limits matter [S108].
   - Mitigations: preregister the analysis and keep a frozen held-out stream [S114].
8. **Rough size [Inf].** About 20 repositories × 25 tasks × 7 conditions × 3 seeds ≈ 10k rollouts for a full run. A development slice of about 5 repositories × 10 tasks is manageable.

**Other gaps:**
- **Provenance and attribution correctness** isn't scored by any benchmark found beyond source-target credit [S124].
- **Uncertainty versus verified labels** isn't benchmarked for coding.
- **Cross-machine sync and conflict semantics** (concurrent writers) appear only in formal or simulation work [S26][S65].
- **Write-time token accounting** isn't standardized.
- **Repeat-failure prevention** (does memory stop agents retrying known-bad fixes?) is proposed but not built [S43].

## 6. Open questions

1. At what corpus size, model capability and price point does a structured store beat **raw log plus agentic grep**? Evidence conflicts [S39][S40][S120]. Does the crossover move as models improve (the "bitter lesson" of [S39])?
2. Which representation transfers best for coding: abstract insights, concrete traces or executable skills? Do the results conflict, or do they depend on whether transfer is within a repository or across domains [S53][S113][S14]?
3. Can applicability checking be made cheap and reliable enough to remove memory-induced harm [S113]?
4. How should hivemind resolve contradictions among agents with different trust levels without an LLM judge on the write path [S26]?
5. What is the right default for sharing: pull-only, gated push [S62], or blackboard [S60][S61]? What does each cost in tokens per success?
6. How should "verified" be defined for coding memories (CI pass, merged PR, human review), and how should it decay when dependencies change?
7. How do we stop memory from becoming a channel for self-propagating misalignment [S88] or supply-chain attacks [S89] while staying useful?
8. Does consolidation after the fact ("sleep-time") preserve detail better than extracting at write time [S37][S97]?
9. How much of the reported progress on LoCoMo and LongMemEval survives fixes to scoring targets, judges and dataset cleaning [S94][S124]?
10. **Unverified in this review:**
    - full results tables for most 2026 preprints;
    - licenses and sizes for MemBench, BEAM, MEMTRACK, MemoryArena, Evo-Memory, VibeMemBench, MemDev-Bench, CoMemBench, GroupMemBench, STALE, ConvoMem, LongMemEval-V2 and MPBench;
    - Context-Bench's size;
    - peer-review outcomes for many preprints;
    - any independent replication of the SWE-bench memory gains [S48][S50][S51].
    
    Follow-up: read those papers' experiment sections and dataset cards, and check OpenReview for decisions.

## 7. Sources (all checked 2026-11-10)

1. CoALA, https://arxiv.org/abs/2309.02427
2. Zhang et al., memory mechanism survey, https://arxiv.org/abs/2404.13501
3. Du et al., Rethinking Memory in LLM based Agents, https://arxiv.org/abs/2505.00675
4. Wu et al., From Human Memory to AI Memory, https://arxiv.org/abs/2504.15965
5. Hu et al., Memory in the Age of AI Agents, https://arxiv.org/abs/2512.13564
6. Huang et al., Survey of Agent Memory in the Second Half, https://arxiv.org/abs/2602.06052
7. Zhou et al., Self-Evolving Coding Agents, https://arxiv.org/abs/2608.03392
8. Lin et al., Long-Term Memory Security survey, https://arxiv.org/abs/2604.16548
9. Jiang et al., Adaptation of Agentic AI, https://arxiv.org/abs/2512.16301
10. Luo et al., From Storage to Experience, https://arxiv.org/abs/2605.06716
11. Generative Agents, https://arxiv.org/abs/2304.03442
12. MemGPT, https://arxiv.org/abs/2310.08560
13. Reflexion, https://arxiv.org/abs/2303.11366
14. Voyager, https://arxiv.org/abs/2305.16291
15. MemoryBank, https://arxiv.org/abs/2305.10250
16. HippoRAG, https://arxiv.org/abs/2405.14831
17. HippoRAG 2, https://arxiv.org/abs/2502.14802 (HTML: https://arxiv.org/html/2502.14802v2)
18. GraphRAG, https://arxiv.org/abs/2404.16130
19. LightRAG, https://arxiv.org/abs/2410.05779
20. RAPTOR, https://arxiv.org/abs/2401.18059
21. When to use Graphs in RAG, https://arxiv.org/abs/2506.05690 (HTML: https://arxiv.org/html/2506.05690v3)
22. GraphRAG-Bench (Xiao et al.), https://arxiv.org/abs/2506.02404
23. RAG vs. GraphRAG, https://arxiv.org/abs/2502.11371
24. Zep, https://arxiv.org/abs/2501.13956 (HTML: https://arxiv.org/html/2501.13956v1)
25. AriGraph, https://arxiv.org/abs/2407.04363
26. TOKI, https://arxiv.org/abs/2606.06240
27. MemTxn, https://arxiv.org/abs/2607.27834
28. Hindsight, https://arxiv.org/abs/2512.12818
29. Mem0, https://arxiv.org/abs/2504.19413 (HTML tables: https://arxiv.org/html/2504.19413v1)
30. A-MEM, https://arxiv.org/abs/2502.12110
31. MemoryOS, https://arxiv.org/abs/2506.06326
32. MemOS, https://arxiv.org/abs/2507.03724
33. MIRIX, https://arxiv.org/abs/2507.07957
34. Memory-R1, https://arxiv.org/abs/2508.19828
35. Mem-α, https://arxiv.org/abs/2509.25911
36. MemAgent, https://arxiv.org/abs/2507.02259
37. LightMem, https://arxiv.org/abs/2510.18866
38. SmartSearch, https://arxiv.org/abs/2603.15599
39. MemoryTax, https://arxiv.org/abs/2610.29823
40. Wolff et al., Cost and Accuracy of LTM in Distributed MAS, https://arxiv.org/abs/2601.07978
41. Zep blog, "Is Mem0 Really SOTA in Agent Memory?", https://www.getzep.com/blog/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/
42. Letta blog, "Benchmarking AI Agent Memory: Is a Filesystem All You Need?", https://www.letta.com/blog/benchmarking-ai-agent-memory
43. PROJECTMEM, https://arxiv.org/abs/2606.12329
44. Agent Workflow Memory, https://arxiv.org/abs/2409.07429
45. ExpeL, https://arxiv.org/abs/2308.10144
46. Dynamic Cheatsheet, https://arxiv.org/abs/2504.07952
47. ACE, https://arxiv.org/abs/2510.04618 (HTML: https://arxiv.org/html/2510.04618v3)
48. ReasoningBank, https://arxiv.org/abs/2509.25140 (HTML: https://arxiv.org/html/2509.25140v2)
49. MemP, https://arxiv.org/abs/2508.06433
50. SWE-Exp, https://arxiv.org/abs/2507.23361 (HTML: https://arxiv.org/html/2507.23361v2)
51. Agent KB, https://arxiv.org/abs/2507.06229
52. Memento, https://arxiv.org/abs/2508.16153
53. Memory Transfer Learning, https://arxiv.org/abs/2604.14004
54. Do Agent Optimizers Compound?, https://arxiv.org/abs/2607.14004
55. Evaluating AGENTS.md, https://arxiv.org/abs/2602.11988
56. Do Context Files Help Coding Agents?, https://arxiv.org/abs/2607.27250
57. G-Memory, https://arxiv.org/abs/2506.07398
58. Collaborative Memory, https://arxiv.org/abs/2505.18279
59. INMS: Memory Sharing, https://arxiv.org/abs/2404.09982
60. Blackboard LLM MAS, https://arxiv.org/abs/2507.01701
61. Blackboard MAS for data discovery, https://arxiv.org/abs/2510.01285
62. SEDM, https://arxiv.org/abs/2509.09498
63. Intrinsic Memory Agents, https://arxiv.org/abs/2508.08997
64. PatchBoard, https://arxiv.org/abs/2605.29313
65. Token Coherence, https://arxiv.org/abs/2603.15183
66. Handoff Debt, https://arxiv.org/abs/2606.02875
67. The Handoff Tax, https://arxiv.org/abs/2608.24358
68. Lost in the Middle, https://arxiv.org/abs/2307.03172
69. Chroma, Context Rot, https://www.trychroma.com/research/context-rot
70. LLMLingua, https://arxiv.org/abs/2310.05736
71. LongLLMLingua, https://arxiv.org/abs/2310.06839
72. LLMLingua-2, https://arxiv.org/abs/2403.12968
73. The Complexity Trap, https://arxiv.org/abs/2508.21433 (HTML: https://arxiv.org/html/2508.21433v3)
74. RAG or Long-Context LLMs? (Self-Route), https://arxiv.org/abs/2407.16833
75. Long Context vs. RAG: An Evaluation and Revisits, https://arxiv.org/abs/2501.01880
76. Anthropic, Managing context on the Claude Developer Platform, https://www.anthropic.com/news/context-management (redirects to claude.com)
77. Cursor, Improving agent with semantic search, https://cursor.com/blog/semsearch
78. MINJA, https://arxiv.org/abs/2503.03704
79. AgentPoison, https://arxiv.org/abs/2407.12784
80. PoisonedRAG, https://arxiv.org/abs/2402.07867
81. SpAIware (Embrace The Red), https://embracethered.com/blog/posts/2024/chatgpt-macos-app-persistent-data-exfiltration/
82. MEXTRA, https://arxiv.org/abs/2502.13172
83. Memory poisoning systematic study / MPBench, https://arxiv.org/abs/2606.04329
84. MemPoison, https://arxiv.org/abs/2605.29960
85. eTAMP, https://arxiv.org/abs/2604.02623
86. ShadowMerge, https://arxiv.org/abs/2605.09033
87. MAPLE-Guard, https://arxiv.org/abs/2608.00426
88. Self-Propagating Misalignment, https://arxiv.org/abs/2610.04083
89. Package Hallucination Attacks via Rule Files, https://arxiv.org/abs/2610.09264
90. The Memory Trust Gap, https://arxiv.org/abs/2609.01852
91. LoCoMo, https://arxiv.org/abs/2402.17753
92. LoCoMo project page, https://snap-research.github.io/locomo/
93. LongMemEval, https://arxiv.org/abs/2410.10813 (HTML: https://arxiv.org/html/2410.10813v2)
94. LongMemEval repo (cleaned 2025-09; MIT), https://github.com/xiaowu0162/LongMemEval
95. MemoryAgentBench, https://arxiv.org/abs/2507.05257 (HTML: https://arxiv.org/html/2507.05257v4)
96. MemoryAgentBench repo README (ICLR 2026 note), https://github.com/HUST-AI-HYZ/MemoryAgentBench
97. HaluMem, https://arxiv.org/abs/2511.03506
98. MEMTRACK, https://arxiv.org/abs/2510.01353
99. BEAM, https://arxiv.org/abs/2510.27246
100. MemBench, https://arxiv.org/abs/2506.21605
101. Multi-Session Chat ("Beyond Goldfish Memory"), https://arxiv.org/abs/2107.07567
102. Letta, Context-Bench, https://www.letta.com/blog/context-bench
103. LoCoBench-Agent, https://arxiv.org/abs/2511.13998
104. SWE-bench, https://arxiv.org/abs/2310.06770
105. OpenAI, Introducing SWE-bench Verified, https://openai.com/index/introducing-swe-bench-verified/
106. SWE-Bench Pro, https://arxiv.org/abs/2509.16941
107. Terminal-Bench (2.0), https://arxiv.org/abs/2601.11868
108. Benchmarks as Software / Terminal-Bench 2.1, https://arxiv.org/abs/2611.08417
109. SWE-Bench-CL, https://arxiv.org/abs/2507.00014
110. Evo-Memory, https://arxiv.org/abs/2511.20857
111. MemoryArena, https://arxiv.org/abs/2602.16313
112. VibeMemBench, https://arxiv.org/abs/2609.23570
113. MemDev-Bench, https://arxiv.org/abs/2610.27524
114. DreamBench-SWE, https://arxiv.org/abs/2608.20664
115. LongMemEval-V2, https://arxiv.org/abs/2605.12493
116. GateMem, https://arxiv.org/abs/2606.18829
117. CoMemBench, https://arxiv.org/abs/2609.32192
118. GroupMemBench, https://arxiv.org/abs/2605.14498
119. STALE, https://arxiv.org/abs/2605.06527
120. ConvoMem, https://arxiv.org/abs/2511.10523
121. MemoryBench (THUIR), https://arxiv.org/abs/2510.17281
122. SWE-rebench, https://arxiv.org/abs/2505.20411
123. EvalMem, https://arxiv.org/abs/2609.22231
124. Same Ranking, Different Winner (TIAP), https://arxiv.org/abs/2605.24060
125. AI Agents That Matter, https://arxiv.org/abs/2407.01502
126. τ-bench, https://arxiv.org/abs/2406.12045
127. Holistic Agent Leaderboard (HAL), https://arxiv.org/abs/2510.11977
128. Agentic Benchmark Checklist (ABC), https://arxiv.org/abs/2507.02825
129. Adding Error Bars to Evals, https://arxiv.org/abs/2411.00640
130. Judging LLM-as-a-Judge, https://arxiv.org/abs/2306.05685
131. SWE-Bench+, https://arxiv.org/abs/2410.06992
132. The SWE-Bench Illusion, https://arxiv.org/abs/2506.12286
133. LoCoMo repo (CC BY-NC 4.0; 10 conversations), https://github.com/snap-research/locomo
134. LongMemEval cleaned dataset, https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned
135. MemoryAgentBench repo (MIT), https://github.com/HUST-AI-HYZ/MemoryAgentBench
136. HaluMem repo (CC BY-NC-ND 4.0), https://github.com/MemTensor/HaluMem
137. MemBench repo (no license detected), https://github.com/import-myself/Membench
138. SWE-bench repo (MIT), https://github.com/SWE-bench/SWE-bench
139. SWE-Bench Pro public repo (MIT), https://github.com/scaleapi/SWE-bench_Pro-os
140. Terminal-Bench repo (Apache-2.0; now harbor-framework/terminal-bench-1), https://github.com/harbor-framework/terminal-bench-1 ; leaderboard https://www.tbench.ai/
141. SWE-Bench-CL repo (MIT), https://github.com/thomasjoshi/agents-never-forget
142. τ-bench repo (MIT), https://github.com/sierra-research/tau-bench
143. MemoryBench repo (MIT), https://github.com/THUIR/MemoryBench
144. GraphRAG-Benchmark repo (MIT), https://github.com/GraphRAG-Bench/GraphRAG-Benchmark
145. GateMem repo (MIT), https://github.com/rzhub/GateMem
146. DreamBench-SWE repo (Apache-2.0), https://github.com/iroiro147/dreambench-swe
147. LoCoBench-Agent repo (Apache-2.0), https://github.com/SalesforceAIResearch/LoCoBench-Agent
148. letta-evals / Context-Bench (Apache-2.0), https://github.com/letta-ai/letta-evals ; leaderboard https://leaderboard.letta.com
149. Context Rot code (MIT), https://github.com/chroma-core/context-rot
150. OpenReview venue lookups (AWM ICML 2025; G-Memory NeurIPS 2025 spotlight; Mem0 ECAI 2025; Voyager TMLR; MemoryBank AAAI 2024; Memory-R1 ACL 2026; MINJA NeurIPS 2025; AgentPoison NeurIPS 2024; AI Agents That Matter TMLR; LightRAG ICLR 2025 withdrawn; Collaborative Memory submitted to ICLR 2026; Agent KB ICLR 2026 withdrawn / KnowFM@ACL 2026; MemGPT, MemoryOS, SWE-Exp, Evo-Memory CoRR only), https://api2.openreview.net/notes/search?term=<title>&source=forum
151. Repo READMEs for venues: https://github.com/joonspk-research/generative_agents (UIST '23); https://github.com/noahshinn/reflexion (NeurIPS 2023); https://github.com/LeapLabTHU/ExpeL (AAAI 2024); https://github.com/parthsarthi03/raptor (ICLR bibtex); https://github.com/OSU-NLP-Group/HippoRAG (NeurIPS '24, ICML '25)
152. Mem0 paper HTML (tables 1–2; token-metric definition), https://arxiv.org/html/2504.19413v1
153. System repo licenses via GitHub API: https://github.com/mem0ai/mem0 (Apache-2.0), https://github.com/getzep/graphiti (Apache-2.0), https://github.com/microsoft/graphrag (MIT), https://github.com/HKUDS/LightRAG (MIT), https://github.com/letta-ai/letta (Apache-2.0), https://github.com/MemTensor/MemOS (Apache-2.0), https://github.com/BAI-LAB/MemoryOS (Apache-2.0), https://github.com/google-research/reasoning-bank (Apache-2.0), https://github.com/WujiangXu/A-mem-sys (MIT)

**Note on author attributions** in §2: entries such as "Wolff & ?" (S40) and "Gao & Zhang?" (S59) should read "Wolff et al." and "Gao et al." Only first authors and author counts were verified for these.
