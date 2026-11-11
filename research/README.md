# Research reports

Nine research threads, each run on 2026-11-10 against public primary sources:
repositories at pinned commits, official docs and specs, papers, and issue
trackers. Each report is saved verbatim as its research agent produced it. A
few were delivered in parts and joined here. Synthesis and decisions live in
[`docs/`](../docs/), and these reports are the evidence underneath.

Every substantive claim carries a label:

- **[V]** verified in a primary source;
- **[C]** a project, vendor, or author claim;
- **[I]** independent evidence;
- **[Inf]** the researcher's inference;
- **[?]** unknown.

Treat numbers as the sources' numbers: vendor benchmarks are leads, not proof.

| # | Report | Question | Headline finding |
|---|---|---|---|
| 01 | [Rust agent-memory systems](01-agent-memory-systems-rust.md) | Memorose, Aionforge, AndaDB/KIP, Statelet, HelixDB, memvid, funes, and others: what do they actually do? | None is ready to adopt. Aionforge and KIP have the best semantics; funes is the LLM-free episodic baseline to beat. All benchmarks are self-reported |
| 02 | [Storage and retrieval foundations](02-storage-and-retrieval-foundations.md) | Which embedded stores and indexes could we build on, and how durable are they? | Durability defaults are often weak. Exact vector search is feasible to about 1M per scope with quantization. No Jepsen evidence for any candidate |
| 03 | [Memory frameworks](03-memory-frameworks.md) | Graphiti/Zep, Mem0, Letta, and 20 others: semantics, write cost, evidence | Graphiti has the best temporal model at 8–19 LLM calls per episode. Mem0 v2 dropped updates. Independent studies show simple baselines match memory systems |
| 04 | [Coding-agent memory in practice](04-coding-agent-memory-practice.md) | How do Copilot, Claude Code, Codex, Cursor, Devin, and OSS tools do memory and coordination? | One shape everywhere: a tiny index plus details. Only Copilot checks citations. Stale files override code 90% of the time. 12 evaluation scenarios |
| 05 | [Literature and benchmarks](05-research-and-benchmarks.md) | What does research show, and how should we evaluate? | Annotated bibliography; benchmark matrix; baseline ladder; tiered reuse plan; a sketch of the missing shared-memory coding benchmark |
| 06 | [Architecture, distribution, protocols, security](06-architecture-distribution-protocols-security.md) | Concurrency, sync, consensus, MCP and gRPC, poisoning | Log as truth; one daemon per store; MCP 2026-07-28 is stateless; don't rely on push; poisoning is demonstrated in production |
| 07 | [Embeddings and local inference](07-embeddings-and-local-inference.md) | Which models and runtimes for a lean local service? | Lexical search always on; `bge-small` int8 as the default hybrid; model2vec as the pure-Rust fallback; version every embedding space |
| 08 | [Computational hippocampus](08-computational-hippocampus.md) | Which hippocampal theories yield testable engineering mechanisms? | Cards as keys, evidence as values; boundary episodes; salience as weight, not gate; gain × need consolidation; 9 rejected metaphors |
| 09 | [Engine design techniques](09-engine-design-techniques.md) | How to build our own all-Rust engine correctly | "Bitcask with segments"; one record per commit; fatal fsync errors; torn tail vs corruption; simulated-filesystem crash testing |
