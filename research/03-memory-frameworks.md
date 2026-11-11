# Research 03: Agent-memory frameworks and services

> Research thread report, checked 2026-11-10. Produced by a research agent from public sources and saved verbatim. Evidence labels: **[V]** verified in a primary source, **[C]** project or vendor claim, **[I]** independent evidence, **[Inf]** inference, **[?]** unknown. Synthesis and decisions live in [`docs/`](../docs/).

## Executive summary

**Conclusion:** none of these projects meets the whole hivemind brief. That brief is Rust-native, lean enough for a laptop or usable as a service, shared by many agents, with provenance and supersession, at low write cost. Better semantics could be borrowed from several of them. On coding tasks, the independent evidence that any memory system beats simple baselines is weak.

- **Graphiti** has the strongest temporal model I could verify in code:
  - Raw episodes, plus facts that carry real-world validity (`valid_at`/`invalid_at`) and system time (`created_at`/`expired_at`).
  - Each fact lists the episodes it came from.
  - Contradictions retire facts rather than delete them.
  - It costs about **8–19 LLM calls and roughly 20–45k input tokens per chat-message episode** (my estimate from code paths and prompt sizes).
  - One LLM judgment per edge decides contradictions, and a user measured it failing on cheap non-reasoning models (issue #1666).
  - The MCP server's write queue lives only in memory, and silent episode drops are reported (#1707).
  - Its supported backends are FalkorDB, Neo4j and Neptune. Kuzu support is deprecated because Kuzu itself was archived in October 2025.
- **Mem0 v2.0.0 (April 2026)** dropped its ADD/UPDATE/DELETE/NOOP update step and its graph memory.
  - It now makes one LLM call per write, adds only, and deduplicates by exact hash.
  - Contradictions pile up by design (#4896). The OSS code at this commit does not store the "linked memory" IDs the maintainers say handle this.
  - The OpenMemory MCP server was sunset and removed; Mem0's MCP is now hosted only.
- **Letta** retired its V1 server, including shared memory blocks with optimistic locking.
  - Letta Code stores memory as git-backed Markdown (MemFS), and background "dreaming" edits it in git worktrees.
  - Shared memory repositories exist, but only for agents hosted on Letta Cloud.
- **The coding-agent tools are converging on files.** Letta Code, Claude Code, Codex (whose memory pipeline is in Rust) and Microsoft Agent Framework all use a `MEMORY.md` index plus topic files.
- **Vendor benchmark claims are unreliable.**
  - The Mem0 vs Zep LoCoMo dispute included an arithmetic error: Zep's 84% became 75.14%, and Mem0's re-run put Zep at 58.44%.
  - LoCoMo's own data has open reports of wrong answer keys.
  - In Mem0's own paper, simply passing the full conversation (72.9%) beats every memory system.
  - Newer claims (Mem0 92.5/94.4, Hindsight 91.4, MemOS 88.8) are unreplicated.
- **Independent preprints, mostly from 2026, point one way:**
  - How memories are retrieved matters more than how they are written.
  - Storing raw chunks with zero LLM calls matches LLM fact extraction.
  - Plain RAG matched Mem0 at 8.4× lower total cost of ownership, while Graphiti and Cognee scored 55–56% on LoCoMo.
- **Coding-agent benchmarks are the most relevant evidence:**
  - VibeMemBench: 11 of 12 solver/system pairings fail to beat running with no memory.
  - MemDev-Bench: 29 systems land at 46–53% against 47.8±1.5% with no memory, and a plain SQLite full-text-search baseline scores 50.0%.
- **"Just use X" options:**
  - Basic Memory (local, MCP, Markdown plus SQLite; AGPL; built around one user).
  - Hindsight (MIT, embedded Postgres, MCP per memory bank; LLM calls on write).
  - Graphiti MCP (best semantics; heaviest).
- **Recommended for hivemind:**
  - Immutable episodes, with derived facts that carry both time axes and explicit "supersedes" links.
  - Writes authored by the agent itself, so no extra LLM calls, with optional background consolidation.
  - Hybrid retrieval with no LLM in the loop.
  - Durable, ordered writes within each scope.
  - Applicability metadata (repo, commit, dependency versions).
  - Our own coding-agent eval, with no-memory, full-text-search and raw-trajectory baselines.

**Report path (intended):** `/Users/mingley/dev/hivemind/research/03-memory-frameworks.md` (not written; full content below)

---

## 03 — Agent-memory frameworks and services: semantics, costs, evidence

*Part of the hivemind research. Facts were checked 2026-11-10 unless noted, using public sources only.*

**Labels**
- **[V]**: verified in a primary source (code, docs, release notes or paper text).
- **[C]**: a claim by the project or vendor.
- **[I]**: independent third-party evidence. Most of it is arXiv preprints that have not been peer-reviewed, and some authors build competing systems; these are marked **[I\*]**.
- **[Inf]**: my inference.
- **[?]**: unknown.

**Method**
- **Read directly at pinned commits:** Graphiti, Mem0, Letta, and the benchmark disputes.
  - graphiti `ebef986a0b23`
  - mem0 `f1791daa86d4`, plus tag `v1.0.0`
  - letta `archive` branch `56ba9c25552`
  - letta-code `186d75b570aa`
  - Sources cited as `G:`, `M:`, `LA:` and `LC:` below are relative to these commits.
- **Brief sections:** facts were gathered by two delegated, read-only research passes on the same date, against the commits listed in the Sources.
- **Spot-checks:**
  - Repo metadata for 17 repos.
  - Supermemory's closed-source server.
  - Cognee's default graph provider.
  - The crates in Codex's memories pipeline.
  - The lock file in the MCP memory server.
- **Not done:** I did not clone, build or run any code.

---

## 1. Summary

1. **Write cost falls into three tiers in the default configuration.**
   - **No LLM calls per write:** Letta Code MemFS, the Claude memory tool and Claude Code memory, Basic Memory, the MCP reference memory server, OpenAI Agents SDK sessions, and LangGraph Store.
   - **About one call per write or chunk:** Mem0 v2, Cognee, Hindsight, and the LlamaIndex fact block.
   - **Multi-call pipelines:**
     - Graphiti: about 8–19 calls per chat-message episode [Inf].
     - MIRIX: one routing call plus up to six agent loops.
     - Memobase: three calls per buffer flush.
2. **Graphiti has the most complete temporal and provenance model that I could verify in code.**
   - Episodes hold the raw source.
   - Facts (edges) carry `valid_at`/`invalid_at` (when the fact was true in the world) and `created_at`/`expired_at` (when the system recorded or retired it).
   - Each fact keeps a list of the episodes that produced it.
   - When a contradiction is found, old facts are invalidated and never deleted.
3. **Graphiti's contradiction handling is fragile.**
   - It rests on one small-model LLM judgment per edge.
   - That judgment was measured collapsing on non-reasoning small models (#1666).
   - Default search does not filter out expired facts.
4. **The Graphiti MCP server is not durable.**
   - It accepts writes into an in-process asyncio queue per `group_id` and replies "queued" before any processing.
   - Silent drops are reported (#1707).
   - Its supported backends are FalkorDB (the default, with an embedded `falkordblite` option), Neo4j and Neptune. Kuzu is deprecated because Kuzu itself is archived.
5. **Mem0 v2.0.0 (2026-04-16) removed both its update step and its graph memory.**
   - The ADD/UPDATE/DELETE/NOOP update phase and graph memory are gone.
   - A write is now one LLM call, ADD-only, with exact-hash dedup and hybrid retrieval.
   - Contradictions accumulate by design (#4896).
   - OpenMemory (Mem0's local MCP server) was sunset and removed on 2026-07-29.
6. **Letta retired its V1 server.**
   - V1 had shared memory blocks protected by optimistic locking, and a per-block history.
   - Development moved to Letta Code, which keeps memory as git-backed Markdown (MemFS, also called "context repositories").
   - Shared memory repositories exist, but only for cloud-hosted agents.
7. **Coding-agent tools are converging on files.**
   - Letta Code, Claude Code auto memory, Codex memories (Rust) and the Microsoft Agent Framework harness all use a `MEMORY.md` index plus topic files.
   - The files are written by the agent itself or by a background consolidator, and are usually versioned.
8. **The LoCoMo results are disputed and all vendor-run.**
   - Mem0's paper reported Zep at 65.99.
   - Zep re-ran and claimed 84%, then corrected it to 75.14% after an arithmetic error was pointed out.
   - Mem0's re-run, using Mem0's prompt, gave Zep 58.44%.
   - Letta could not reproduce Mem0's MemGPT numbers.
9. **LoCoMo is a weak instrument.**
   - The conversations fit in a model's context window.
   - In Mem0's own paper, full context beats every memory system.
   - The upstream repo has been unmaintained since August 2024 and has open reports of wrong answer keys.
   - The choice of which stored form gets scoring credit can flip rankings.
10. **Independent preprints agree on three points.**
    - Retrieval quality dominates how memories are written.
    - Raw chunks, with zero LLM calls, match LLM fact extraction.
    - RAG matched Mem0 at 8.4× lower total cost of ownership (TCO), while Graphiti and Cognee trailed at 55–56%.
11. **The coding-agent benchmarks are sobering.**
    - VibeMemBench: 11 of 12 solver/system pairings do not beat running with memory off.
    - MemDev-Bench: 29 systems score 46–53% resolved, against 47.8±1.5% with no memory, and a plain SQLite full-text-search baseline scores 50.0%.
12. **Recent headline numbers are all self-reported.**
    - Mem0 claims 92.5 on LoCoMo and 94.4 on LongMemEval; Hindsight 91.4 on LongMemEval; MemOS 88.8; Mastra 94.9; Supermemory claims "#1".
    - Supermemory's engine is closed source.
    - Hindsight's "independent reproduction" involved paper co-authors.
13. **No candidate meets hivemind's full brief.**
    - Basic Memory, Hindsight and Graphiti MCP come closest; each has a disqualifying gap (see §5).
    - The semantics worth adopting are listed in §5.

---

## 2. In-depth

### 2.1 Graphiti (getzep/graphiti) and Zep

**Status and maturity [V]**
- Apache-2.0, about 32.7k stars, 560 open issues, not archived ([gh api](https://github.com/getzep/graphiti)).
- Latest release v0.31.2 (2026-10-20). v0.31.0 (2026-10-14) made `gpt-6-luna` the default OpenAI model.
- Commits land daily.
- The README now says Graphiti "is a framework for building graphs", while Zep is "the unified context layer for enterprises" built on its own graph database service, Konig. It also says "Neither is an agent memory product" (G:README.md#L80-L109).
- The `getzep/zep` repo is now "Examples, Integrations, & More" rather than a self-hostable server ([gh api](https://github.com/getzep/zep)).

**Data model [V]**

| Element | Fields and behaviour | Source |
|---|---|---|
| `EpisodicNode` | `source` (message, text or json), `source_description`, `content` (raw text; dropped if `store_raw_episode_content=False`), `valid_at` (the source's reference time), `created_at`, `entity_edges`, `episode_metadata` | G:graphiti_core/nodes.py#L318-L332; graphiti.py#L218, #L1034 |
| `EntityNode` | `name`, `labels`, `summary` (at most 1,000 chars), typed `attributes`, `name_embedding` | nodes.py#L499-L502; utils/text_utils.py#L26 |
| `EntityEdge` (the "fact") | `name` (relation type), `fact` (prose), `fact_embedding`, `episodes` (list of episode UUIDs = provenance), `valid_at`/`invalid_at` (when it was true in the world), `created_at`/`expired_at` (when the system learned and retired it), `reference_time`, `attributes` | edges.py#L49-L54, #L263-L290 |
| `CommunityNode` | Clusters built by label propagation, with LLM-written summaries | utils/maintenance/community_operations.py#L82-L96 |
| `SagaNode` | Named episode sequences, linked by `HAS_EPISODE` and `NEXT_EPISODE` edges | graphiti.py#L984-L1094 |
| `group_id` | Partitions every node and edge | edges.py#L51; nodes.py#L96 |

**Write path: what `add_episode` actually does [V]** (G:graphiti.py#L1334-L1586)

1. **Context.** Loads the group's last 10 episodes (`RELEVANT_SCHEMA_LIMIT = 10`). No LLM call (graphiti.py#L1434-L1446; search/search_utils.py#L64).
2. **`extract_nodes`.** One LLM call on the default model. The prompt depends on the source type (node_operations.py#L73-L163, #L286-L329).
3. **`resolve_extracted_nodes`.**
   - For each node it gathers up to 15 candidates (`NODE_DEDUP_CANDIDATE_LIMIT`), by identity properties, exact name and embedding search with cosine ≥ 0.6.
   - It then resolves deterministically where it can.
   - Nodes still unresolved go to **one batched LLM call** (node_operations.py#L66-L68, #L649-L815, #L816-L910).
4. **`extract_edges`.** One LLM call with `max_tokens = 16384`. It returns facts plus optional `valid_at`/`invalid_at` (edge_operations.py#L134-L260, #L166, #L310-L371).
5. **Dangling endpoints.** Any edge endpoints that were not extracted as nodes may need one more node-dedup LLM call (graphiti.py#L866-L914).
6. **`resolve_extracted_edges`.**
   - Exact duplicates are removed without an LLM.
   - Each fact is embedded, then run through two hybrid searches (BM25 plus cosine, fused with RRF): one for duplicates with the same endpoints, one group-wide for invalidation candidates.
   - Per edge, `_dedupe_extracted_edge` either takes an exact-match fast path or makes **one `resolve_edge` call on the small model**. That call returns `duplicate_facts` and `contradicted_facts`.
   - A new edge with no timestamps gets **one more `extract_timestamps` call** (small model).
   - Custom edge types that have fields get one attribute call.
   - Source: edge_operations.py#L424-L646, #L786-L831, #L864-L1089.
7. **Temporal invalidation.** Deterministic, no LLM (edge_operations.py#L715-L783). Details are under the supersession semantics below.
8. **`extract_attributes_from_nodes`.**
   - One small-model call per node whose entity type has fields (node_operations.py#L985-L1086).
   - Summaries: if a node's summary plus its new facts fits in 2,000 chars, the facts are appended without an LLM call. Otherwise nodes are summarised in batches of 30 per call (node_operations.py#L1088-L1270, #L1134).
9. **Persist.** Bulk write plus embeddings, optional saga edges, and optional `update_communities`, which adds LLM summary calls per affected node (graphiti.py#L984-L1094, #L1532-L1545).

**Call count [Inf, derived from the code above]:** calls ≈ 2 + d + Σ over edges (r + t + a) + N_typed + ⌈S/30⌉, plus communities if enabled.
- d: 0–1 node-dedup calls.
- r, t, a: 0–1 each per edge, for resolution, timestamps and attributes.
- N_typed: number of nodes with a typed entity type.
- S: number of nodes whose summary needs an LLM rewrite.

**What that means for a typical chat message [Inf]** (about 5 entities and 5 facts, on a non-empty graph):
- About 8–12 calls with untyped entities.
- About 13–19 calls with the MCP server's default ontology. That ontology has 10 built-in entity types with typed attributes, which the README notes are enabled by default (mcp_server/config/config.yaml#L96-L115; mcp_server/README.md#L241).

**Token estimate [Inf]**
- I measured the template text of each prompt from its source:

  | Prompt | Approx. template tokens |
  |---|---|
  | `extract_nodes.extract_message` | 1.8k |
  | `extract_edges.edge` | 1.6k |
  | `extract_attributes` | 0.9k |
  | `resolve_edge` | 0.8k |
  | `dedupe_nodes` | 0.6k |
  | `extract_timestamps` | 0.4k |

- The node extraction, node dedup, edge extraction and attribute prompts also embed the current episode plus up to 10 previous ones.
- With ~300-token episodes, that is roughly **20–25k input tokens per episode untyped, and 35–45k with typed attributes**.
- Graphiti ships a `token_tracker` (graphiti.py#L413-L422; llm_client/token_tracker.py). Measure with it before relying on these figures.
- A user report gives about 36 s per record with bulk ingestion, and about 12 s per message as the expected `add_episode` latency (#1262) [I, anecdotal].

**Defaults [V]**
- LLM: `gpt-6-luna` for both `model` and `small_model` (llm_client/openai_base_client.py#L35-L36).
- Embedder: OpenAI.
- Cross-encoder: an OpenAI logprob reranker on `gpt-4.1-mini` (cross_encoder/openai_reranker_client.py#L31-L32).
- Graph driver: Neo4j, if none is given (constructor docstring, graphiti.py#L210-L337).
- **GLiNER2Client** is an option: a local extraction model of about 205M–340M parameters handles entity extraction, and other prompts still go to an LLM (llm_client/gliner2_client.py).
- An open issue asks for an `add_episode` that skips LLM extraction entirely ([#1299](https://github.com/getzep/graphiti/issues/1299)).

**Supersession semantics [V]**
- **The judge prompt** says: "Only mark a fact as contradicted when the NEW FACT and the existing fact cannot both be true". It also forbids inferring a contradiction from recency or from status updates (prompts/dedupe_edges.py).
- **What happens after a contradiction** (edge_operations.py#L715-L783):
  - The older, contradicted edge gets `invalid_at = new.valid_at` and `expired_at = now`.
  - If a contradicting candidate *starts later* than the new edge, the **new edge** is expired instead, so late-arriving old facts become history.
  - An edge that arrives with an `invalid_at` already set is expired immediately.
  - Open bug: an edge whose `invalid_at` is in the future is created already expired ([#2012](https://github.com/getzep/graphiti/issues/2012)).
- **Duplicates** reuse the existing edge and append the new episode's UUID to its `episodes`, so one fact can have several sources (edge_operations.py#L864-L1089).
- **Nothing is deleted** by invalidation. `SearchFilters` supports `valid_at`, `invalid_at`, `created_at` and `expired_at` (search/search_filters.py#L55-L73).
  - All filters default to `None`, so expired facts come back in results unless the caller filters them [Inf from code].
  - The MCP `search_memory_facts` tool exposes valid/invalid date bounds (mcp_server/src/graphiti_mcp_server.py#L615-L694).
- **Nodes** have no validity window, and their summaries are overwritten [Inf].

**Retrieval [V]**
- Search methods: cosine, BM25 and BFS for edges and nodes; BM25 for episodes.
- Rerankers: RRF, MMR, `node_distance`, `episode_mentions` and `cross_encoder` (search/search_config.py#L32-L118).
- `Graphiti.search()` defaults to `EDGE_HYBRID_SEARCH_RRF`, which uses no LLM, or to `EDGE_HYBRID_SEARCH_NODE_DISTANCE` when a center node is given (graphiti.py#L1913-L1972).

**Backends and footprint [V]**
- Supported: Neo4j 5.26, FalkorDB 1.1.2 (plus embedded `falkordblite`, which needs Python 3.12+), and Amazon Neptune with OpenSearch Serverless for full-text search (G:README.md#L160-L226).
- Kuzu 0.11.2 is "deprecated… upstream Kuzu project is no longer maintained". `kuzudb/kuzu` is archived, last pushed 2025-10-10 ([gh api](https://github.com/kuzudb/kuzu)).
- The MCP server supports only FalkorDB and Neo4j. The default is a combined FalkorDB + MCP container (mcp_server/README.md#L28-L32, #L112-L114, #L344).
- Open and recent bugs: the FalkorDB driver races when one driver object is shared across groups ([#1676](https://github.com/getzep/graphiti/issues/1676), fixed with request-scoped clients; [#1795](https://github.com/getzep/graphiti/issues/1795) still open). Neo4j vector search was brute-force O(N) ([#1793](https://github.com/getzep/graphiti/issues/1793)).

**Concurrency [V]**
- **Library rule:** "each episode is added sequentially and awaited before adding the next" (graphiti.py#L1404-L1414).
- **`SEMAPHORE_LIMIT` defaults disagree:**
  - Core default is 20, and it caps each `semaphore_gather` call separately rather than globally (helpers.py#L38-L47; graphiti.py#L257-L266).
  - The README says 10 (README.md#L262-L270).
  - The MCP server uses 10 (graphiti_mcp_server.py#L96).
- **MCP queueing:**
  - `QueueService` runs one in-memory `asyncio.Queue` per `group_id`: sequential within a group, parallel across groups.
  - Errors are logged and the episode is dropped (mcp_server/src/services/queue_service.py).
  - `add_memory` returns "queued for processing" before any work happens (graphiti_mcp_server.py#L402-L536).
  - Silent data loss is reported in [#1707](https://github.com/getzep/graphiti/issues/1707).
- **Across processes [Inf]:** there is no coordination. Dedup is a read-then-write, so two server processes writing the same group can race.

**MCP server [V]**
- Tools: `add_memory`, `search_nodes`, `search_memory_facts`, `delete_entity_edge`, `delete_episode`, `get_entity_edge`, `get_episodes`, `summarize_saga`, `build_communities`, `add_triplet`, `get_episode_entities`, `clear_graph`, `get_status` (graphiti_mcp_server.py#L402-L1146).
- Transports: HTTP (the default, at `/mcp/`) and stdio. The default `group_id` is `"main"` (mcp_server/README.md#L169, #L347-L348).

**Evidence**

**Zep paper** ([arXiv 2501.13956](https://arxiv.org/abs/2501.13956)) [V text; C as vendor].
- **DMR:**
  - Zep scored 94.8% against MemGPT's 93.4% (gpt-4-turbo).
  - But a full-conversation baseline scored 94.4% (gpt-4-turbo) and 98.0% (gpt-4o-mini), against Zep's 98.2%.
  - The paper itself calls DMR inadequate (60 messages per conversation).
- **LongMemEval_s:**

  | Answer model | Full context | Zep | Context size | Latency |
  |---|---|---|---|---|
  | gpt-4o-mini | 55.4% | 63.8% | — | — |
  | gpt-4o | 60.2% | 71.2% | 115k → 1.6k tokens | 28.9 s → 2.58 s |

  Zep did worse on single-session-assistant questions: −17.7% relative with gpt-4o.
- **Setup:** the numbers come from Zep's cloud service, not OSS Graphiti. Graphs were built with gpt-4o-mini, with BGE-m3 for embedding and reranking. Latency was measured from a laptop to AWS.
- **Not reported:** ingestion cost.

**Zep's later LoCoMo post** [C] ([blog](https://www.getzep.com/blog/the-retrieval-tradeoff-what-50-experiments-taught-us-about-context-engineering/)).
- About 80%, with p50 retrieval under 200 ms.
- Going from 5/2 to 20/20 (edge/node limits) adds about 10.4 points for about 4× the tokens.

**Independent evidence**
- [arXiv 2601.07978](https://arxiv.org/abs/2601.07978) [I]: OSS Graphiti and Cognee reach 55–56% on LoCoMo, against 77–81% for mem0, RAG and full context. The authors attribute the gap to incomplete retrieval.
- The Mem0 paper reports Zep at 65.99 [I\*, competitor].
- [MemoryTax, arXiv 2610.29823](https://arxiv.org/abs/2610.29823) [I] includes Zep, but on one conversation with one run per cell.

### 2.2 Mem0 (mem0ai/mem0)

**Status [V]**
- Apache-2.0, about 68.8k stars, 932 open issues; last push 2026-11-06.
- Python SDK v2.2.1 (2026-09-25); Node SDK ts-v3.3.1 ([gh api](https://github.com/mem0ai/mem0)).
- **Version naming is confusing.** The PyPI package is 2.x, but the migration guide is titled "oss-v2-to-v3" and a maintainer calls it "our v3 SDK" ([v2.0.0 release](https://github.com/mem0ai/mem0/releases/tag/v2.0.0); [#4896](https://github.com/mem0ai/mem0/issues/4896)).

**v1 algorithm (the paper and SDK ≤ 1.x) [V]**
- **Paper description:**
  - An extraction LLM proposes candidate facts, using a conversation summary, recent messages and the new exchange.
  - An update step then compares them with similar existing memories, and an LLM chooses **ADD / UPDATE / DELETE / NOOP** through a tool call ([arXiv 2504.19413](https://arxiv.org/abs/2504.19413)).
- **SDK v1.0.0: 2 LLM calls per `add`:**
  - Fact extraction ([main.py@v1.0.0#L434](https://github.com/mem0ai/mem0/blob/v1.0.0/mem0/memory/main.py#L434)).
  - One batched update call ([#L497-L502](https://github.com/mem0ai/mem0/blob/v1.0.0/mem0/memory/main.py#L497-L502)).
- **With graph memory on, 3 more calls ran in parallel:** entity extraction, relation extraction and the delete decision ([graph_memory.py@v1.0.0#L195-L345](https://github.com/mem0ai/mem0/blob/v1.0.0/mem0/memory/graph_memory.py#L195-L345)). That makes about 5 calls for Mem0g.

**v2 algorithm (current) [V]**
- **One LLM call** using `ADDITIVE_EXTRACTION_PROMPT` (M:mem0/memory/main.py#L920-L960; M:mem0/configs/prompts.py#L468).
  - Inputs: the last 10 session messages and the top-10 existing memories. Memory IDs are mapped to integers "anti-hallucination".
  - Output: `{text, attributed_to, linked_memory_ids}`.
  - Relative dates are resolved against an "Observation Date".
- **After the call:**
  - The new memories are embedded in one batch.
  - MD5 exact-hash dedup runs against the retrieved top-10 and within the batch (main.py#L1008-L1040).
  - The memories are persisted, and `ADD` rows are written to history.
  - spaCy (no LLM) extracts entities into a separate `{collection}_entities` collection (main.py#L583-L730, #L1100-L1200).
- **Other paths:**
  - `infer=False`: 0 LLM calls (main.py#L881-L915).
  - Procedural memory: 1 call (main.py#L2007-L2050).
- **Defaults:**
  - LLM `gpt-5-mini` (mem0/llms/openai.py#L40).
  - Vector store Qdrant (mem0/vector_stores/configs.py#L9).
  - History in SQLite with a threading lock (configs/base.py#L42-L44; memory/storage.py#L14-L15).
- **Search** fuses semantic, lemmatised BM25 and entity boosts (main.py#L1642-L1790).
  - Default changes in 2.0.0: `top_k` 100 → 20, `threshold` None → 0.1, `rerank` True → False.
  - Graph memory removed: "~4,000 lines of graph driver code" (release notes).

**Contradictions [V]**
- The code does nothing about them.
- Maintainers say v3 "handles contradictions by design through the extraction prompt and memory linking" ([#4896](https://github.com/mem0ai/mem0/issues/4896), closed as intended).
- However, at `f1791da` `_add_to_vector_store` persists only `data`, `text_lemmatized`, `hash`, the timestamps and `attributed_to`. **The LLM's `linked_memory_ids` are never read** (main.py#L1015-L1040).
- [Inf] Linking is therefore not implemented in the OSS add path; it may exist in the Platform.

**Other semantics [V]**
- **Temporal:** `created_at`/`updated_at`, plus `expiration_date`, which hides memories from search (main.py#L427-L449, #L769-L789). There are no validity intervals.
- **Provenance:** a history table (old memory, new memory, event) and a raw messages table per session scope. Memories carry no source-message IDs [Inf].
- **Scopes:** `user_id` / `agent_id` / `run_id`, which must be passed inside `filters` on search. Agent-scoped writes get a prompt suffix (main.py#L942-L946).

**Concurrency issues (open) [V]**
- Hash dedup has a check-then-write race that creates duplicates: [#6515](https://github.com/mem0ai/mem0/issues/6515) (Python) and [#6531](https://github.com/mem0ai/mem0/issues/6531) (TS).
- The vector is persisted even when the history write fails, so a retry creates duplicates ([#7879](https://github.com/mem0ai/mem0/issues/7879)).
- TS dedup searches a hardcoded top-10 with no similarity threshold ([#7123](https://github.com/mem0ai/mem0/issues/7123)).

**MCP [V]**
- OpenMemory was sunset ([#4923](https://github.com/mem0ai/mem0/issues/4923), 2026-04-22) and removed ([PR #6530](https://github.com/mem0ai/mem0/pull/6530), 2026-07-29). It is archived as a snapshot in `mem0ai/openmemory#5`.
- The `mem0ai/openmemory` repo now hosts an unrelated CLI for porting coding sessions.
- **Mem0 MCP is hosted only:** "Nothing runs on your machine" (M:docs/platform/mem0-mcp.mdx).
- The supported self-hosted path is `server/`: FastAPI + Postgres/pgvector + Neo4j (PR #6530).
- Plugins for Claude Code, Codex and Cursor live in `integrations/*/core/mcp_server.py`. Which backend they call is [?].

**Evidence**

**Mem0 paper** [V text; baselines run by Mem0, a competitor].
- LoCoMo overall J score (LLM-as-judge):

  | System | J score |
  |---|---|
  | Full context | 72.90 |
  | Mem0g | 68.44 |
  | Mem0 | 66.88 |
  | Zep | 65.99 |
  | best RAG | ~61.0 |
  | LangMem | 58.10 |
  | OpenAI | 52.90 |
  | A-Mem | 48.38 |

- p95 total latency: 1.44 s for Mem0 against 17.1 s for full context.
- Tokens retrieved per query: Mem0 1,764; Mem0g 3,616; Zep 3,911; full context 26,031.
- Memory-store footprint per conversation claimed: Mem0 about 7k tokens, Mem0g about 14k, Zep "in excess of 600k".
- Mem0 says Zep only answered well after "several hours" of background graph building.

**Current README claims** [C].
- LoCoMo 92.5, LongMemEval 94.4, BEAM-1M 64.1, BEAM-10M 48.6, at about 7k tokens per query with top-200 retrieval.
- These are "Mem0's managed platform, which includes proprietary optimizations not available in the open-source SDK" (M:README.md#L53-L80).
- The harness is open ([mem0ai/memory-benchmarks](https://github.com/mem0ai/memory-benchmarks)). Its default answerer and judge are gpt-4o.

**Independent evidence**
- [2603.02473](https://arxiv.org/abs/2603.02473) [I]: write strategy (Mem0-style extraction vs raw chunks vs summarisation) moves accuracy only 3–8 points, while retrieval method moves it 20. Raw chunks, with zero LLM calls, match or beat extraction.
- [2603.04814](https://arxiv.org/abs/2603.04814) [I]: long-context GPT-5-mini beats a Mem0-based system on LongMemEval and LoCoMo. The memory system becomes cheaper after about 10 turns at 100k context.
- [2601.07978](https://arxiv.org/abs/2601.07978) [I]: mem0 sits in the 77–81% cluster with RAG, but RAG has 8.4× lower TCO.
- [2608.11879](https://arxiv.org/abs/2608.11879) [I]: comparing Mem0, Hindsight and Mastra OM, accuracy spans 21–54%; whether and when they break even is highly system- and backbone-dependent.
- [VibeMemBench 2609.23570](https://arxiv.org/abs/2609.23570) [I]: the confidence interval for glm-5 + Mem0 lies entirely below the memory-off baseline.
- [MemoryTax 2610.29823](https://arxiv.org/abs/2610.29823) [I, one conversation]: Mem0 beats full context only with gpt-4o-mini, and its build costs more than a whole full-context rollout.
- [2605.04897](https://arxiv.org/abs/2605.04897) [I\*]: Mem0 scores 61.4% on LoCoMo with a matched gpt-4.1-mini answerer. The authors propose a competing system.

### 2.3 Letta (MemGPT)

**Status [V]**
- `letta-ai/letta` README: "The current source code lives in letta-ai/letta-code". The `archive` branch "contains the retired Letta V1 API server".
- Last release 0.16.8 (2026-05-14).
- `letta-code`: TypeScript, Apache-2.0, v0.34.51 (2026-11-10), 342 open issues.
- Letta Cloud is the default backend, and a local backend is available (LC:README.md#L62-L68).

**V1 model (archive) [V]**
- **Core memory** is a set of blocks with `label`, `value`, `limit`, `description` and `read_only`. Block limits default to 100,000 characters, and persona/human blocks to 20,000 (LA:letta/schemas/block.py#L13-L36; LA:letta/constants.py#L433-L435).
- **Tools** (LA:letta/functions/function_sets/base.py#L87-L520):
  - Block editing: `core_memory_append`, `core_memory_replace`, `memory_insert`, `memory_replace`, `memory_rethink`, `memory_apply_patch`.
  - Archival memory: `archival_memory_insert` and `archival_memory_search` (tagged vector passages).
  - Recall memory: `conversation_search` over message history.
- **Shared blocks:**
  - A many-to-many `blocks_agents` table links blocks to agents.
  - **Optimistic locking** uses SQLAlchemy's `version_id_col`.
  - A `BlockHistory` table records each change with `actor_type`, `actor_id` and a sequence number (LA:letta/orm/block.py#L53-L69; LA:letta/orm/block_history.py#L12-L40).
- **Storage:** Postgres (pgvector) or SQLite (LA:letta/settings.py).
- **Write cost [Inf]:** there is no extraction pipeline. Each memory edit is a tool step inside the agent's own loop. Sleep-time agents rewrite memory in the background.

**Current model: Letta Code [V]** ([MemFS docs](https://docs.letta.com/concepts/memfs); [shared memory docs](https://docs.letta.com/concepts/shared-memory))
- **MemFS:** each agent has its own git repo.
  - Files at the root are pinned into the system prompt.
  - Directories with their own `MEMORY.md` index stay out of context until needed.
  - Every edit is committed. Cloud agents push to a hosted repo; local-only agents keep the repo on the machine.
- **Search:** no vector index by default. An optional `memfs-search` mod gives keyword search, with semantic or hybrid search via QMD.
- **Concurrent editing:** dreaming and reflection subagents edit in **git worktrees** and merge back ([context-repositories blog](https://www.letta.com/blog/context-repositories)).
- **Shared memory repositories:** org-owned git repos that can be attached to many agents.
  - Agents commit and push; others pull.
  - "Shared memory repositories require cloud-hosted agents."
  - The legacy shared-block API still works, but "users should migrate".

**Sleep-time compute** ([arXiv 2504.13171](https://arxiv.org/abs/2504.13171)) [V abstract; C results]
- About 5× less test-time compute at equal accuracy on Stateful GSM-Symbolic and Stateful AIME.
- Up to +13% and +18% accuracy when sleep-time compute is scaled.
- 2.5× lower cost per query when amortised across related queries.
- How well it works tracks how predictable the queries are.
- This is not a memory-retrieval benchmark. Letta now calls the feature "dreaming" (LC:README.md#L126).

**Letta's benchmark critiques [C]**
- **"Is a filesystem all you need?"** ([blog, Aug 2025](https://www.letta.com/blog/benchmarking-ai-agent-memory)):
  - Conversation history was placed in files, and the agent got `grep`, `search_files` and open/close tools.
  - Result: **74.0% on LoCoMo with gpt-4o-mini**, against Mem0g's reported 68.5%.
  - Letta's argument: agent capability matters more than the retrieval mechanism, and LoCoMo measures retrieval rather than agentic memory.
- **Mem0's MemGPT numbers:** Letta asked how they were produced in [mem0#3004](https://github.com/mem0ai/mem0/issues/3004). The issue was closed with no answer in the thread [V].
- **Context-Bench** ([blog](https://www.letta.com/blog/context-bench)):
  - Uses fictional, SQL-generated data with `open_files` and `grep_files` tools.
  - Sonnet 4.5 led at launch with 74.0% at $24.58.
  - It measures the model's context engineering, not a memory store.

**Independent evidence**
- I found no independent evaluation of Letta Code's MemFS [?].
- The direction is supported by [LongMemEval-V2 2605.12493](https://arxiv.org/abs/2605.12493): storing trajectories as files and letting a coding agent gather evidence scored best (72.5%, against 48.5% for the best RAG baseline), at higher latency [I; written by the LongMemEval authors].

### 2.4 Benchmark disputes and replications

**Timeline [V]**

| Date | Event | Source |
|---|---|---|
| 2025-01-20 | Zep paper (DMR and LongMemEval) | arXiv 2501.13956 |
| 2025-04-28 | Mem0 paper: Zep 65.99 vs Mem0g 68.44 | arXiv 2504.19413 |
| May 2025 | Zep blog alleges Mem0 misconfigured Zep: (1) both speakers treated as "user"; (2) timestamps appended to text instead of using `created_at`; (3) searches run sequentially. Zep initially claims 84% | [Zep blog](https://www.getzep.com/blog/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/) |
| 2025-05-08 | Mem0's CTO shows the 84% counted correct category-5 answers in the numerator but excluded category-5 from the denominator, and objects to Zep's prompt changes. His re-run gives Zep **58.44 ± 0.20** | [zep-papers#5](https://github.com/getzep/zep-papers/issues/5) |
| 2025-05-12 | Zep corrects to **75.14 ± 0.17** over 10 runs, and defends the prompt change as clarifying timestamp semantics. Issue closed 2025-05-19 | same |
| 2025-06-21 → Aug 2025 | Letta asks about the MemGPT numbers (#3004); Letta's filesystem agent scores 74.0% | links above |
| Dec 2025 | Zep reports about 80% at under 200 ms. The blog's banner misdates this "December 9, 2015" | Zep blog |
| 2026 | Mem0 claims 92.5 / 94.4 on its managed platform | Mem0 README |

**Interpretation [Inf]**
- Every number in this dispute was produced by a vendor, with vendor-chosen prompts, answer models and ingestion setups.
- Gaps of 5–17 points come from exactly those choices.
- Mem0's own table shows full context (72.9) beating every memory system on LoCoMo.

**LoCoMo data quality [V]** ([snap-research/locomo](https://github.com/snap-research/locomo))
- The original paper reports about 300 turns and about 9k tokens per conversation on average ([arXiv 2402.17753](https://arxiv.org/abs/2402.17753)); Zep cites 16–26k tokens.
- The repo was last pushed 2024-08-13.
- Open ground-truth error reports:
  - [#27](https://github.com/snap-research/locomo/issues/27): 3 label errors (wrong field, wrong day, wrong speaker).
  - #35: two more conv-26 errors plus ambiguous items.
  - #43: mislabelled category-5 questions.
  - [#44](https://github.com/snap-research/locomo/issues/44): a third-party audit finds 5 gold-answer errors and 2 annotation defects.
  - #45: invalid evidence IDs.
- Category-5 (adversarial) questions store their answer under `adversarial_answer` (#11, #41).
- The issues are also used to post leaderboard claims (#26, #31, #34, #38, #39).
- MemoryTax found that its one miss was a wrong answer key, and that photo-caption evidence advantages agents [I].

**Methodology papers [I]**
- [2605.24060](https://arxiv.org/abs/2605.24060): changing only which stored form gets credit (raw, source or canonical) changes nDCG on 83–94% of queries and flips orderings for Mem0 and MemoryOS.
- [2610.09193](https://arxiv.org/abs/2610.09193): on MemoryAgentBench's conflict-resolution split, a frozen "last write wins" rule scores 80.25%, and 67 gold answers cannot be reached under last-write semantics.
- [2603.02473](https://arxiv.org/abs/2603.02473): retrieval, not write strategy, dominates.

**Independent comparisons [I]**

| Study | Systems | Setting | Finding |
|---|---|---|---|
| [2601.07978](https://arxiv.org/abs/2601.07978) | mem0, Graphiti, cognee, RAG, full context | LoCoMo; cloud–edge testbed measuring CPU, RAM and I/O | mem0, RAG and full context at 77–81%; Graphiti and cognee at 55–56%; RAG has 8.4× lower TCO than mem0 |
| [2603.02473](https://arxiv.org/abs/2603.02473) | 3 write strategies × 3 retrieval methods | LoCoMo | Spread is 57.1–77.2% across retrieval methods but 3–8 points across write strategies; raw chunks are competitive |
| [2603.04814](https://arxiv.org/abs/2603.04814) | Mem0-based system vs long context | LongMemEval, LoCoMo, PersonaMemv2 | Long context is more accurate on LongMemEval and LoCoMo; memory is cheaper after about 10 turns at 100k |
| [2608.11879](https://arxiv.org/abs/2608.11879) | Mem0, Hindsight, Mastra OM | Up to 400 turns, 665 LoCoMo questions | Accuracy 21–54%; serving cost is not predictable from conversation size; break-even ranges from tens of turns to never |
| [2610.29823](https://arxiv.org/abs/2610.29823) | Full context, BM25, Mem0, Zep vs Pi, Codex, Claude Code | 1 conversation, 1 run per cell | Memory helps only weak models; a strong model is capped by the weaker model that wrote the memory; coding agents lead the cost/accuracy frontier |
| [VibeMemBench 2609.23570](https://arxiv.org/abs/2609.23570) | Mem0, SimpleMem, MemoryOS, A-MEM | 111 SWE-rebench targets | Injecting verified experience directly gives +1.1 to +4.5 points; 11 of 12 system pairings fail to beat memory-off |
| [MemDev-Bench 2610.27524](https://arxiv.org/abs/2610.27524) | A-Mem, Mem0, MemOS, Supermemory, Claude-Mem, plus 29 more in an appendix | Real repo histories, Claude Code | 29 systems at 46.0–52.67% resolved vs no memory 47.78 ± 1.54, and an SQLite FTS baseline at 50.0 (single runs); raw trajectories cut steps; failures come from reusing memory without reassessing whether it applies |
| [2605.04897](https://arxiv.org/abs/2605.04897) [I\*] | True Memory (the authors' system) vs Mem0, Supermemory, Zep, EverMemOS | LoCoMo, LongMemEval, BEAM | A single SQLite file with verbatim events and multi-stage retrieval reaches 93.0% on LoCoMo; argues extracting at ingestion is the wrong primitive |

---

## 3. Brief sections: claimed vs implemented vs demonstrated

These facts come from the delegated passes at the pinned commits listed in the Sources, unless I verified them myself.

**Cognee (topoteretes/cognee)**
- **Maturity [V]:** Apache-2.0, v1.6.7 (2026-11-06), about 33.2k stars.
- **Data model [V]:** `DataPoint` graph nodes carrying `version`, `source_*` fields and an importance weight.
- **Default stack [V]:** fully embedded:
  - Graph: Ladybug, which I spot-checked: `graph_database_provider` defaults to `"ladybug"`.
  - Vectors: LanceDB.
  - Relational: SQLite.
- **Writes [V]:** `cognify` makes **one structured-output LLM call per chunk**. The default model is `openai/gpt-5.6-luna`. It runs in batches, with a background flag.
- **Supersession:** none on the write path. An opt-in `review_conflicts` LLM pipeline labels facts `current`, `superseded` or `conflicting`. `temporal_cognify` is deprecated.
- **Provenance and access:** a provenance ledger exists but is off by default. Multi-tenant access control is on by default in the API.
- **MCP [V]:** `cognee-mcp`, over stdio, SSE or HTTP.
- **Benchmarks:** BEAM numbers are self-reported [C]. Independent: 55–56% on LoCoMo in 2601.07978 [I]; 50.67% on MemDev-Bench (single run) [I].
- **Known issues:** concurrent LanceDB writers corrupt ingestion ([#5254](https://github.com/topoteretes/cognee/issues/5254)); Ladybug segfaults with a corrupted write-ahead log ([#6170](https://github.com/topoteretes/cognee/issues/6170)).

**LangMem plus the LangGraph BaseStore**
- **Store data model [V]:** an `Item` is a namespace tuple, a key, a JSON value, and `created_at`/`updated_at`. Operations are put, get, search and `list_namespaces`; a delete is a put of `None`.
- **Backends [V]:** `InMemoryStore` ("lost when the process exits"), `PostgresStore` (pgvector, TTL, `ON CONFLICT DO UPDATE`, so last writer wins), and `SqliteStore`.
- **LangMem writes [V]:**
  - About **1–2 LLM calls per invocation**, through trustcall, plus 1 per extra "phase" and 1 if a query model is set.
  - There is no default model.
  - Memories are patched in place. Deletes are off by default, and the docstring contradicts that ([#192](https://github.com/langchain-ai/langmem/issues/192)).
  - There is no versioning, timestamps beyond the two above, or source links.
- **Scopes:** hierarchical namespace tuples, with `("memories","{langgraph_user_id}")` as the default.
- **Docs vs code [C]:** semantic, episodic and procedural memory are categories in the docs; procedural memory is implemented as prompt optimisation.
- **Maturity:** no GitHub releases; PyPI 0.0.30 (2025-10). No MCP.
- **Benchmarks:** Mem0's paper reports J = 58.10 with p95 search latency of 59.8 s [I\*].

**MemOS (MemTensor/MemOS)**
- **Maturity [V]:** Apache-2.0, v2.0.36 (2026-11-09).
- **Data model [V]:** a MemCube holds text, activation (KV-cache), parametric and preference memory.
  - Text memories carry `status` (activated, resolving, archived or deleted), `version`, and a `history` of archived versions with `update_type` (conflict, duplicate, extract, unrelated or feedback).
  - Each memory has `sources` with `message_id`, `chat_time` and `doc_path`.
  - `visibility` can be private, public or session.
- **Writes [V]:** by default the write stores raw ~1k-token windows with no LLM call. A background step then makes **1 LLM call per chat window**, plus further calls I did not count (dedup, preferences, skills, reorganiser). Default model `gpt-4o-mini`.
- **Self-hosting:** needs Neo4j and Qdrant. The local plugin uses SQLite FTS5 [C].
- **MCP [V]:** FastMCP over stdio, http or sse.
- **Benchmarks:** LoCoMo 88.83 and LongMemEval 89.20 [C], with no reproduction. 17.5% resolved… no — MemOS appears in MemDev-Bench's main five-system study [I]; see that paper for its numbers.
- **Known issue:** dedup is exact-string only ([#2409](https://github.com/MemTensor/MemOS/issues/2409)).

**Memobase (memodb-io/memobase)**
- **Maturity [V]:** Apache-2.0; latest tag v0.0.42; last commit 2026-01-11, so activity looks stalled [Inf].
- **Data model:** a user profile of topic and subtopic slots, plus an event timeline.
- **Writes [V]:**
  - A per-user buffer flushes at 1,024 tokens or after 1 hour idle.
  - Each flush makes **3 base LLM calls** (summary, topic extraction, merge), plus conditional ones. Models are `gpt-4o-mini` and `o4-mini`.
  - The merge step can add, update or delete profile slots, and overwrites in place.
- **Provenance:** raw blobs are "not found once you flush" [V].
- **Footprint:** Postgres plus Redis. MCP over SSE or stdio.
- **Benchmarks:** LoCoMo 75.78% [C]. Disputes about these numbers are in issues [#79](https://github.com/memodb-io/memobase/issues/79) and #101.

**MIRIX (Mirix-AI/MIRIX)**
- **Maturity [V]:** Apache-2.0, v0.1.6.
- **Data model:** six typed memory stores in Postgres/pgvector, with an optional Neo4j graph. Rows carry `occurred_at`, `actor` and `source_refs`.
- **Writes [V]:**
  - An `add` is queued (in memory, or Kafka).
  - A meta-agent LLM routes the batch to up to 6 memory managers, each running its own tool loop: **at least 1 + k calls** per add.
- **Concurrency:** a per-agent `asyncio.Lock` that only works within one process.
- **MCP:** MIRIX is an MCP *client* only.
- **Benchmarks:** LoCoMo 85.4% [C]. A user measured 0.62 ([#6](https://github.com/Mirix-AI/MIRIX/issues/6)) [I].

**A-MEM (arXiv 2502.12110)**
- **Repos [V]:** WujiangXu/A-mem is the evaluation code, WujiangXu/A-mem-sys is the system, and agiresearch/A-mem is a third copy. All MIT.
- **Data model:** Zettelkasten-style notes in an in-process dict plus Chroma.
- **Writes [V]:**
  - A-mem-sys makes **2 calls per add**: content analysis, then "evolution", which rewrites neighbouring notes in place.
  - In agiresearch/A-mem, `analyze_content` is never called, so an add costs 1 call ([#10](https://github.com/agiresearch/A-mem/issues/10), [#3](https://github.com/agiresearch/A-mem/issues/3)).
- **Missing:** no scopes, no provenance, no delete, no concurrency control.
- **Benchmarks:** Mem0's re-run gave J = 48.38 [I\*]. Reproductions differ in [WujiangXu/A-mem#28](https://github.com/WujiangXu/A-mem/issues/28).

**Supermemory (supermemoryai/supermemory)**
- **The engine is closed [V, spot-checked]:** the repo is MIT, but "the downloadable self-hosted server binary is built from a separate, non-public codebase".
- **Consequence:** its update semantics (`updates`/`extends`/`derives` relations, an `isLatest` flag, background "dreaming") cannot be audited [C].
- **MCP:** hosted.
- **Benchmarks:** claims "#1 on LongMemEval, LoCoMo, ConvoMem" [C]. MemDev-Bench evaluates it among its five main systems [I].
- **Known issues:** failed ingestion discards content ([#1827](https://github.com/supermemoryai/supermemory/issues/1827)); unbounded memory growth leads to OOM ([#1826](https://github.com/supermemoryai/supermemory/issues/1826)).

**Hindsight (vectorize-io/hindsight)**
- **Maturity [V]:** MIT, v1.2.1 (2026-11-10). About 60.3k stars despite being created 2025-10-30, which is a lead only.
- **Data model:** memory banks hold world facts, experiences, consolidated observations and mental models.
- **Writes [V]:**
  - `retain` makes **1 LLM call per 3,000-char chunk**. It extracts what/when/where/who/why, `occurred_start`/`occurred_end` plus a `mentioned_at` time, and causal links.
  - Background consolidation packs 8 facts per call and judges each as `unrelated`, `repeats`, `changes` or `ends`, keeping the prior state as history.
  - Observations track `source_memories` and a `proof_count`.
  - Default model `gpt-4o-mini`, with local bge-small embeddings.
- **Footprint [V]:** embedded Postgres (`pg0`) by default.
- **MCP [V]:** one endpoint per bank at `/mcp/{bank_id}/`.
- **Benchmarks:**
  - LongMemEval 91.4% and LoCoMo up to 89.61% [C].
  - The "independent reproduction" includes paper co-authors [Inf].
  - Included in 2608.11879, and 51.33% on MemDev-Bench (single run) [I].
- **Known issues:** a mental-model refresh used "46M tokens in ~10h" ([#6471](https://github.com/vectorize-io/hindsight/issues/6471)); every append rewrites all prior units ([#6241](https://github.com/vectorize-io/hindsight/issues/6241)).

**Basic Memory (basicmachines-co/basic-memory)**
- **Maturity [V]:** **AGPL-3.0**, v1.0.0 (2026-10-29).
- **Data model [V]:** Markdown files are the source of truth, indexed in SQLite (WAL mode) with sqlite-vec and local FastEmbed bge-small.
  - Notes contain observations (`- [category] text #tag`) and wikilink relations.
  - Optional temporal qualifiers such as `@effective[...)`; relative dates are rejected.
  - `created_by` / `last_updated_by` columns, and a `locked: true` frontmatter flag.
- **Writes:** **no LLM calls** [Inf, consistent with code]. Edits are precise (append, find-replace with an expected count, section replace).
- **Coding-agent hooks [V]:** Claude Code and Codex hooks append to an append-only local log; durable notes are written later.
- **MCP [V]:** native, over stdio or HTTPS.
- **Gaps:** no automatic supersession. Team sharing is a hosted cloud offering [C].
- **Known issues:** duplicate rows in the search index ([#1771](https://github.com/basicmachines-co/basic-memory/issues/1771)); concurrent starts time out ([#1731](https://github.com/basicmachines-co/basic-memory/issues/1731)).

**MCP reference "memory" server (modelcontextprotocol/servers/src/memory)**
- **Data model [V]:** entities (name, type, observations[]) and relations, stored in JSONL.
- **Search [V]:** substring search only.
- **Writes [V]:** no LLM. Each mutation rewrites the whole file to a temp file and renames it over the original.
- **Concurrency [V, spot-checked]:** a cross-process `<file>.lock` created with O_EXCL. This was added only after data-loss bugs ([#4797](https://github.com/modelcontextprotocol/servers/issues/4797), [#5058](https://github.com/modelcontextprotocol/servers/issues/5058)).
- **Missing:** timestamps, versioning and scopes.
- **License:** moving from MIT to Apache-2.0.
- **Usefulness:** a useful *minimum baseline*, not a candidate.

**txtai (neuml/txtai)**
- **Maturity [V]:** Apache-2.0, v9.14.0.
- **Data model:** an embeddings database (sparse plus dense) with a graph and SQL.
- **Agent memory [V]:** an in-process deque per session, not persisted.
- **Writes:** no LLM calls. One writer at a time through an `RLock`.
- **Footprint:** the default install pulls in torch and faiss.
- **MCP:** the API can expose itself as MCP. Useful as an index library, not as memory semantics.

**LlamaIndex `Memory`**
- **Maturity [V]:** MIT, 0.14.25. The older buffer classes are deprecated.
- **Short-term memory [V]:** a SQLAlchemy chat store that **defaults to in-memory**.
- **Long-term memory [V]:** priority-ordered blocks, written on flush:
  - `FactExtractionMemoryBlock`: 1 call per flush, plus a condense call above 50 facts.
  - `VectorMemoryBlock`: no LLM calls.
- **Missing:** timestamps, provenance and sharing.
- **Known issue:** concurrent writes to the default store are silently lost ([#23806](https://github.com/run-llama/llama_index/issues/23806)).

**CrewAI unified `Memory` (redesigned 2026-02, #4420)**
- **Maturity [V]:** MIT.
- **Data model [V]:** `MemoryRecord` with a hierarchical `scope` path, `importance`, `source` and `private`.
- **Writes [V]:**
  - 0–2 LLM calls per item (`gpt-6-luna`), on a single background thread.
  - Consolidation can update or delete records. Items above 0.98 similarity are dropped silently.
- **Recall:** blends semantic score with recency (30-day half-life) and importance.
- **Storage [V]:** LanceDB with a file lock, or a Redis lock.
- **Benchmarks:** a third party reported 46.0% on LongMemEval vs 57.6% with no memory, unverified ([#5800](https://github.com/crewAIInc/crewAI/issues/5800)).
- **Known issue:** memory poisoning across agents ([#6043](https://github.com/crewAIInc/crewAI/issues/6043)).

**Microsoft Agent Framework (MAF), Semantic Kernel, Kernel Memory**
- **Maturity [V]:** MAF is MIT, python-1.25.0 and dotnet-1.28.0. Semantic Kernel describes MAF as its successor.
- **Context providers [V]:** Mem0, Redis and Cosmos providers; a .NET `FileMemoryProvider` with a `memories.md` index.
- **Python harness memory [V]:**
  - `MEMORY.md` (200-line limit), plus topics, transcripts and a state file.
  - 1 LLM call per run to extract up to 5 durable memories.
  - Consolidation every 24 hours.
- **Semantic Kernel:** `WhiteboardProvider` makes 1 call per message.
- **Kernel Memory:** now an archived research project under `dluc/` [V].

**OpenAI Agents SDK sessions**
- **Maturity [V]:** MIT, v0.24.0.
- **What it is:** a `Session` protocol that stores raw items. There is no extraction or supersession.
- **Backends:** SQLite (`:memory:` by default), Redis, SQLAlchemy, MongoDB, Dapr, OpenAI Conversations, encrypted, and compaction.
- **LLM calls:** none, except the compaction session (`responses.compact`).
- **Concurrency:** one lock per database path, within a process.
- **Fit:** good as transcript storage, not as memory.

**Other 2025–2026 entrants (brief)**
- **Codex memories** (openai/codex, Apache-2.0, **Rust**) [V, crates spot-checked]:
  - Phase 1 runs in the background and turns each idle rollout into raw memories, with secrets redacted, on `gpt-6-luna`. Rollouts are claimed through leases in a state DB.
  - Phase 2 takes a global lock and runs a consolidation sub-agent with no network access, on `gpt-6.1-terra`.
  - It writes git-baselined `~/.codex/memories/{MEMORY.md, memory_summary.md, skills/}`.
  - Memories are off by default.
  - Issue [#41711](https://github.com/openai/codex/issues/41711): chats with local providers are sent to OpenAI without notice.
- **Claude memory tool and Claude Code** [V docs]:
  - The API tool `memory_20250818` runs client-side: view, create, `str_replace`, insert, delete and rename under `/memories`.
  - Claude Code auto memory is a `MEMORY.md` index plus typed topic files, loading the first 200 lines or 25 KB. It is machine-local, and subagents do not inherit it.
- **Mastra Observational Memory** [V]:
  - Observer and Reflector LLMs (`gemini-2.5-flash`) run in background chunks; each reflection makes a new "generation".
  - LongMemEval 94.87% with gpt-5-mini [C].
- **Memori:** "Advanced Augmentation" is a remote vendor call, even when you bring your own database [V].
- **EverOS** (formerly EverMemOS): Markdown as the source of truth, with SQLite and LanceDB, plus an MCP package [C].
- **LongMemory** (CaviraOSS, formerly OpenMemory): a SQLite engine claiming immutable content, provenance and "temporal truth" [C, not verified].
- **memU:** the host agent does the synthesis; no LLM calls in the service [C].
- **MemMachine:** Neo4j plus SQL, keeps the original episodes, MCP; 0.9169 on LoCoMo [C]; 47.33% on MemDev-Bench [I].
- **ReasoningBank** (ICLR 2026): distils strategies from successful and failed trajectories; a research artifact.
- **projectmem** ([2606.12329](https://arxiv.org/abs/2606.12329)): an append-only, plain-text log of typed events (issues, attempts, fixes, decisions) for coding agents, projected over MCP. Its "precheck" warns about past failures before an edit. Feasibility study only. **Most similar in spirit to hivemind's coding-memory use case.**
- **Archived or stalled:** Acontext (archived), Memobase (stalled), Kernel Memory (archived README).

---

## 4. Tables

### (a) Write-path LLM cost (default configuration)

| System | LLM calls per write | Default model (in code) | Sync / async / batched |
|---|---|---|---|
| Graphiti `add_episode` | ≈ 2 + 0–1 + per edge (0–1 resolve + 0–1 timestamp + 0–1 attribute) + 1 per typed node + ⌈S/30⌉. About 8–19 per chat message, ~20–45k input tokens [Inf] | `gpt-6-luna` (also the small model); reranker `gpt-4.1-mini` | Synchronous in the library ("run as background"); the MCP server uses an in-memory queue per group (async, not durable); bulk mode has a combined extraction call |
| Zep (cloud) | [?] proprietary | [?] | Async (the paper notes background processing) |
| Mem0 v2 OSS | 1 (`infer=True`); 0 (`infer=False`); entity extraction uses spaCy, no LLM | `gpt-5-mini` | Sync in OSS; the Platform `add` is async (`PENDING` + `event_id`) |
| Mem0 v1 (≤ 1.x) | 2; about 5 with graph memory | Varies; the paper used gpt-4o-mini | Sync; graph ran in parallel |
| Letta V1 / Letta Code | 0 pipeline calls; edits are the agent's own tool steps; dreaming/reflection run periodically | The agent's model | Inline, plus background subagents |
| Cognee `cognify` | 1 per chunk (+1 per chunk if `summary_method="llm"`) | `gpt-5.6-luna` | Batched; optional background |
| LangMem | 1–2 per invocation (+ phases, + query model) | none (required argument) | Optional background executor, debounced |
| LangGraph Store `put` | 0 (embedding only if indexed) | — | Sync |
| MemOS | 0 at write time (raw windows), then 1 per chat window in the background, plus uncounted extras | `gpt-4o-mini` | Async scheduler |
| Memobase | 3 per buffer flush (plus conditional calls) | `gpt-4o-mini` / `o4-mini` | Buffered (1,024 tokens or 1 h); async |
| MIRIX | ≥ 1 + k agent loops (k ≤ 6) | none (README example: Gemini) | Queue; async |
| A-MEM | 2 (system repo) or 1 (agiresearch) | `gpt-4o-mini` | Sync |
| Supermemory | [?] (closed engine) | Proprietary / bring your own | Background "dreaming" [C] |
| Hindsight `retain` | 1 per 3,000-char chunk, plus consolidation at 8 facts per call | `gpt-4o-mini` | `retain` sync by default; consolidation in the background |
| Basic Memory | 0 | — (local FastEmbed) | Sync, plus a file watcher |
| MCP memory server | 0 | — | Sync (full-file rewrite) |
| txtai | 0 | — | Sync |
| LlamaIndex `Memory` | Fact block: 1 per flush (+1 condense); vector block: 0 | `Settings.llm` | On flush, concurrent across blocks |
| CrewAI `Memory` | 0–2 per item | `gpt-6-luna` | One background thread |
| MAF harness / SK whiteboard | 1 per run / 1 per message | Configured client | Inline (awaited) |
| OpenAI Agents SDK sessions | 0 (compaction session: `responses.compact` once ≥ 10 items) | `gpt-6-luna` (compaction) | Sync |
| Codex memories | 1 per idle rollout, plus a consolidation agent | `gpt-6-luna` / `gpt-6.1-terra` | Background, leased; off by default |
| Claude memory tool / Claude Code | 0 extra | The agent's model | Inline |
| Mastra OM | Observer + Reflector per token threshold | `gemini-2.5-flash` | Buffered background |

### (b) Temporal handling, provenance, supersession and scopes

| System | Temporal model | Provenance | Supersession / contradiction | Scopes / sharing |
|---|---|---|---|---|
| Graphiti | **Bitemporal on edges** (`valid_at`/`invalid_at` + `created_at`/`expired_at`); episode `valid_at` | Edge → list of episodes; raw episode content kept (optional) | LLM flags contradictions, then deterministic invalidation; never deletes; history not filtered by default | `group_id`; no ACLs [Inf] |
| Mem0 v2 | `created_at`/`updated_at`, `expiration_date`; relative dates resolved in the prompt | History table; raw messages per session; no message IDs on memories | **None** (ADD-only, MD5 dedup); claimed linking not persisted in OSS | user / agent / run (+ app/org/project on the Platform) |
| Mem0 v1 | Same | History table | LLM UPDATE / DELETE (destructive) | Same |
| Letta V1 | Block history (sequence numbers); message timestamps | `BlockHistory` with actor type and id | Agent rewrites blocks; optimistic locking | Blocks shared across agents |
| Letta Code | Git history | Git commits (author and message) | Agent edits files; git merge | Per-agent MemFS; org shared repos (cloud only) |
| Cognee | `created_at`/`updated_at` (temporal mode deprecated) | Chunk → document; `source_*` fields; ledger off by default | Opt-in `review_conflicts` labels | Datasets, users, permissions |
| LangGraph / LangMem | `created_at`/`updated_at`, TTL | None | In-place patch; deletes off by default | Namespace tuples |
| MemOS | `chat_time`; versions with `timespec` | `sources` with `message_id` / `doc_path` | Archived versions with reason codes | user, cube, project; private / public / session visibility |
| Memobase | Event timestamps | Weak (blobs dropped) | LLM merge overwrites the profile | project → user |
| MIRIX | `occurred_at`, `last_modify` | `source_refs`, turn and chunk IDs | Merge / replace tools; `auto_dream` | org, user, agent, session |
| A-MEM | `timestamp`, `last_accessed` | None | Rewrites neighbours in place | None |
| Supermemory | [C] "isLatest", expiry | [C] chunks kept | [C] "updates win" | `containerTag` |
| Hindsight | Event time (`occurred_*`) + mention time | `source_memories`, `proof_count` | Consolidation keeps the prior state as history | bank, tags, tenant |
| Basic Memory | `@effective[...)` qualifiers; file times | `created_by` / `last_updated_by`; files | Manual edits; `locked` flag | Projects / workspaces |
| MCP memory server | None | None | Manual delete | One file |
| LlamaIndex | None | None | Condense rewrites all facts | `session_id` |
| CrewAI | `created_at`, `last_accessed`, recency decay | `source` | Consolidation update / delete | Scope paths, `private` |
| OpenAI sessions | Item order | Raw items | None | `session_id` |
| Codex / Claude Code | File / git times; Claude Code `modified` frontmatter | Rollout summaries (Codex) | Consolidator rewrites | Per user and machine (project-scoped in Claude Code) |

### (c) Deployment footprint and dependencies

| System | Runtime | Required services (default) | Embedded / local | MCP | License | Latest release |
|---|---|---|---|---|---|---|
| Graphiti | Python | Graph DB (FalkorDB, Neo4j or Neptune+OpenSearch) + LLM + embedder | `falkordblite` (Py ≥ 3.12); Kuzu deprecated | Yes (HTTP / stdio; FalkorDB or Neo4j) | Apache-2.0 | v0.31.2, 2026-10-20 |
| Zep | SaaS / BYOC | Konig (proprietary) | No | Yes [C] | Commercial | n/a |
| Mem0 | Python / TS | Vector store (Qdrant default) + SQLite + LLM; spaCy/fastembed optional | Yes (local Qdrant) | Hosted only (OpenMemory removed) | Apache-2.0 | v2.2.1, 2026-09-25 |
| Letta Code | TypeScript (npm) | Letta Cloud (default) or local backend; git | Local backend; shared memory needs cloud | Client | Apache-2.0 | v0.34.51, 2026-11-10 |
| Cognee | Python | None (Ladybug, LanceDB, SQLite) + LLM | Yes | Yes | Apache-2.0 | v1.6.7, 2026-11-06 |
| LangGraph Store / LangMem | Python | Postgres + pgvector (or SQLite / in-memory) | Yes (SQLite) | No | MIT | langgraph 1.2.20; langmem 0.0.30 (PyPI) |
| MemOS | Python (+ npm plugin) | Neo4j + Qdrant (server); SQLite (plugin) | Plugin only | Yes | Apache-2.0 | v2.0.36, 2026-11-09 |
| Memobase | Python | Postgres + Redis | No | Yes | Apache-2.0 | v0.0.42 tag (stalled) |
| MIRIX | Python | Postgres/pgvector + Redis (+ Neo4j) | No | Client only | Apache-2.0 | v0.1.6 |
| A-MEM | Python | Chroma (embedded) | Yes | No | MIT | none |
| Supermemory | Closed binary | Embedded (claimed) | Yes (≤ 10k docs on the lite license) | Hosted | MIT (SDK only) | server-v0.0.9 |
| Hindsight | Python | Embedded Postgres (`pg0`) or external | Yes | Yes (per bank) | MIT | v1.2.1, 2026-11-10 |
| Basic Memory | Python ≥ 3.12 | SQLite + sqlite-vec + FastEmbed | Yes | Yes | **AGPL-3.0** | v1.0.0, 2026-10-29 |
| MCP memory server | Node ≥ 24 | None (JSONL) | Yes | Yes | MIT → Apache-2.0 | v2.3.0 repo-wide, 2026-11-04 |
| txtai | Python | None (faiss / torch) | Yes | Optional | Apache-2.0 | v9.14.0 |
| LlamaIndex | Python | SQL store (in-memory default) | Yes | No | MIT | 0.14.25 |
| CrewAI | Python | LanceDB (+ Redis lock optional) | Yes | No | MIT | 1.15.39 |
| MAF | Python / .NET | Provider-dependent | Yes (file providers) | Hosting package | MIT | python-1.25.0 |
| OpenAI Agents SDK | Python | SQLite / Redis / … | Yes | n/a | MIT | v0.24.0 |
| Codex memories | **Rust** | Local state DB + git directory | Yes | n/a | Apache-2.0 | (part of Codex) |

---

## 5. Implications for hivemind

**Semantics to adopt** (each follows from the findings cited above)

1. **Raw evidence first, derived facts second.**
   - Store immutable episodes: raw messages, tool traces and diffs, with stable IDs.
   - Make every derived assertion point back to the episodes that support it (Graphiti `episodes[]`, MemOS `sources`, Hindsight `source_memories`).
   - Why: independent work shows lossy extraction at write time discards information that retrieval cannot recover (2603.02473, 2605.04897). MemDev-Bench finds agents benefit from the *process* detail that compressed memories omit.
2. **Bitemporal assertions with explicit supersession.**
   - Use `valid_from`/`valid_to` for world time and `recorded_at`/`retired_at` for system time, plus `supersedes` / `superseded_by` links. Never delete silently.
   - Do **not** make an LLM's contradiction judgment the only path:
     - Graphiti's judge is fragile on small models (#1666).
     - Mem0 gave up on update logic altogether (#4896).
     - Coding agents usually *know* when they are replacing a prior fix or decision, so let them supersede explicitly. Offer an optional background judge as a secondary mechanism.
3. **Writes authored by agents, with no extra LLM calls on the hot path.**
   - Letta, Claude Code, Codex and Basic Memory show agents can write memory well.
   - LLM consolidation should be optional, background and batched, as in Codex Phase 1/2, Letta dreaming and Mem0's `infer=False` path.
   - Report write-side tokens in every evaluation.
4. **Hybrid retrieval with no LLM in the loop.**
   - BM25 plus vector plus entity/graph boost, fused with RRF. Mem0 v2 and Graphiti's defaults both converge here.
   - Make "current-only" vs "as-of time T" an explicit read mode; Graphiti returns expired facts unless filtered.
5. **Files as a projection, not necessarily as the store.**
   - Coding agents are strong file and grep users (Letta 74% on LoCoMo; LongMemEval-V2's coding-agent memory; MemoryTax).
   - A `MEMORY.md` index plus topic files, projected from the store, would let hivemind meet the native memory formats of Claude Code, Codex and Letta.
6. **Scopes and visibility.**
   - Hierarchical namespaces (LangGraph tuples, CrewAI scope paths) combined with first-class user / agent / run / project / repo scopes and private / shared visibility (MemOS).
   - Provenance should record the *writing agent*, as Letta's `BlockHistory` does with actor type and id.
7. **Durable, ordered writes per scope; idempotent; optimistic concurrency.**
   - Avoid in-memory queues (Graphiti #1707).
   - Avoid check-then-write dedup races (Mem0 #6515).
   - Use idempotency keys (Supermemory #1627) and version-based compare-and-set (Letta's `version_id_col`).
8. **Applicability metadata for coding memories.**
   - Attach repo, commit range, file paths, toolchain and dependency versions, and the verification state (tested or not).
   - Why: MemDev-Bench attributes regressions to "reusing memory without reassessing its applicability".

**Costs and designs to avoid**
- Per-edge LLM resolution in the synchronous path (Graphiti: about 8–19 calls per episode).
- Agent loops on write (MIRIX).
- Remote "augmentation" calls (Memori).
- Required heavy services (Neo4j + Qdrant + Redis + Postgres) for the local profile.
- Closed engines (Supermemory).
- AGPL, if hivemind is meant to be embedded as a library [Inf; this needs a licensing decision].

**Could we simply use something?**

| Option | What fits | Disqualifiers for the full brief | Verdict |
|---|---|---|---|
| **Graphiti MCP + falkordblite / FalkorDB** | Best verified temporal and provenance semantics; MCP; many agents via `group_id` | Python plus a graph DB; heavy LLM writes (OpenAI defaults); non-durable MCP queue; weak independent LoCoMo results (55–56%); no cross-process write ordering | Use as the **semantic reference** and possibly as an optional "enrichment" backend; not as the core |
| **Basic Memory** | Local-first files + SQLite + MCP; no LLM writes; Claude Code and Codex hooks; provenance columns | AGPL; Python; built around one user (team sharing is a hosted product); no supersession model; no bitemporal assertions | Probably sufficient for one developer *today*; not a shared multi-agent service |
| **Hindsight** | MIT; embedded Postgres; MCP per bank; observations with history and sources; two time axes | LLM on every retain; vendor-run benchmarks; token blow-up reports (#6471); Python service | **Best "service" candidate to prototype against** |
| **Letta Code MemFS / shared repos** | Git-versioned files, progressive disclosure, concurrent editing via worktrees | Tied to the Letta harness; shared memory requires Letta Cloud | Adopt the *pattern* (git-backed projection), not the product |
| **Mem0 OSS v2** | Cheap writes (1 call); hybrid retrieval; many vector backends | ADD-only (contradictions accumulate); no provenance links; MCP is hosted only; dedup races | Not recommended for "trustable" coding memory |
| **Native agent memory** (Claude Code `MEMORY.md`, Codex memories) | Free, already used by the target agents | Per-machine, per-vendor, not shared across agents or tools | Treat as **sources and sinks** that hivemind syncs with, not competitors |

**What would change our mind**
- An independent coding-agent evaluation, in the style of MemDev-Bench or VibeMemBench, showing a specific system beats both no-memory and an SQLite FTS / raw-trajectory baseline by a margin beyond run-to-run noise. Today none does.
- Graphiti gaining a durable queue, a no-LLM ingest path (#1299) and lighter defaults. Its MCP server would then be close to "just use it" for small teams.
- Basic Memory shipping a self-hostable multi-writer server with per-agent provenance and supersession under a permissive license.
- Letta allowing self-hosted shared memory repositories.
- Evidence that LLM-judged invalidation outperforms explicit, agent-authored supersession on coding tasks.

---

## 6. Open questions

1. **Real write costs.** What are Graphiti's real per-episode tokens on coding-agent episodes (tool traces, diffs)? Measure with `TokenTracker` on fixed fixtures; my 20–45k figure is an estimate.
2. **Mem0 Platform vs OSS.** Does the Platform persist `linked_memory_ids` and use them in ranking, given that the OSS path at `f1791da` does not? What are the Platform's "proprietary optimizations"?
3. **Hindsight under multi-agent load.** How do its consolidation and "mental models" behave? Do concurrent retains to one bank serialise correctly?
4. **Letta Code's local backend.** What storage does it use, and could its git-backed MemFS be shared without Letta Cloud (e.g., a self-hosted git remote)?
5. **Basic Memory licensing.** What does AGPL-3.0 imply for embedding or wrapping it? Is there a commercial license?
6. **Supermemory.** Does its closed server binary implement the documented `isLatest` / `updates` semantics? It cannot be verified.
7. **An eval for hivemind.** How should a coding-agent memory eval cover repeat-failure avoidance, handoff completion and dependency-incompatibility recall, with write-side cost included? projectmem (2606.12329), MemDev-Bench and VibeMemBench are starting points.
8. **Supersession policy.** Should "supersedes" be agent-authored only, judged by an LLM, or both with conflict states (MemOS's `resolving`, Cognee's `conflicting`)? What does each cost in mistaken retirements?
9. **Retrieval defaults.** Should default retrieval return only current facts, current facts plus their history, or a time-travel view? How should conflicting writes from different agents be presented (cf. StateFuse 2607.05844, TOKI 2606.06240; abstracts not reviewed)?
10. **Corpus contamination.** The 2026 arXiv corpus includes many very similar memory systems, some posting to benchmark repos' issue trackers. How much weight should LoCoMo-family results carry at all?

---

## 7. Sources

All checked 2026-11-10.

1. Graphiti repo and metadata — https://github.com/getzep/graphiti (`gh api`, releases)
2. Graphiti `graphiti.py` (`add_episode`, constructor, search) — https://github.com/getzep/graphiti/blob/ebef986a0b2360211b65751c111cacb4a7d790eb/graphiti_core/graphiti.py
3. Graphiti node operations — https://github.com/getzep/graphiti/blob/ebef986a0b2360211b65751c111cacb4a7d790eb/graphiti_core/utils/maintenance/node_operations.py
4. Graphiti edge operations (resolution, invalidation) — https://github.com/getzep/graphiti/blob/ebef986a0b2360211b65751c111cacb4a7d790eb/graphiti_core/utils/maintenance/edge_operations.py
5. Graphiti `resolve_edge` prompt — https://github.com/getzep/graphiti/blob/ebef986a0b2360211b65751c111cacb4a7d790eb/graphiti_core/prompts/dedupe_edges.py
6. Graphiti edges / nodes models — https://github.com/getzep/graphiti/blob/ebef986a0b2360211b65751c111cacb4a7d790eb/graphiti_core/edges.py ; …/nodes.py
7. Graphiti search config and filters — https://github.com/getzep/graphiti/blob/ebef986a0b2360211b65751c111cacb4a7d790eb/graphiti_core/search/search_config.py ; …/search_filters.py
8. Graphiti helpers (`SEMAPHORE_LIMIT`, chunking) — https://github.com/getzep/graphiti/blob/ebef986a0b2360211b65751c111cacb4a7d790eb/graphiti_core/helpers.py
9. Graphiti OpenAI defaults and reranker — …/llm_client/openai_base_client.py ; …/cross_encoder/openai_reranker_client.py ; …/llm_client/gliner2_client.py
10. Graphiti README — https://github.com/getzep/graphiti/blob/ebef986a0b2360211b65751c111cacb4a7d790eb/README.md
11. Graphiti MCP server (tools, queue, config) — https://github.com/getzep/graphiti/tree/ebef986a0b2360211b65751c111cacb4a7d790eb/mcp_server
12. Graphiti issues — #1262, #1299, #1666, #1676, #1707, #1793, #1795, #2012 — https://github.com/getzep/graphiti/issues
13. Kuzu repo (archived) — https://github.com/kuzudb/kuzu
14. Zep paper — https://arxiv.org/abs/2501.13956 (HTML v1 for tables)
15. Zep blog, "Is Mem0 Really SOTA" — https://www.getzep.com/blog/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/
16. Zep blog, "The Retrieval Tradeoff" — https://www.getzep.com/blog/the-retrieval-tradeoff-what-50-experiments-taught-us-about-context-engineering/
17. zep-papers issue #5 (LoCoMo dispute) — https://github.com/getzep/zep-papers/issues/5
18. getzep/zep repo — https://github.com/getzep/zep
19. Mem0 repo and metadata — https://github.com/mem0ai/mem0
20. Mem0 `main.py` (v2 add / search) — https://github.com/mem0ai/mem0/blob/f1791daa86d469249c9b002f62644d58a1eb96a0/mem0/memory/main.py
21. Mem0 prompts (`ADDITIVE_EXTRACTION_PROMPT`) — https://github.com/mem0ai/mem0/blob/f1791daa86d469249c9b002f62644d58a1eb96a0/mem0/configs/prompts.py
22. Mem0 defaults — …/mem0/llms/openai.py ; …/mem0/vector_stores/configs.py ; …/mem0/memory/storage.py ; …/mem0/configs/base.py
23. Mem0 v1.0.0 `main.py` and `graph_memory.py` — https://github.com/mem0ai/mem0/blob/v1.0.0/mem0/memory/main.py ; https://github.com/mem0ai/mem0/blob/v1.0.0/mem0/memory/graph_memory.py
24. Mem0 v2.0.0 release notes — https://github.com/mem0ai/mem0/releases/tag/v2.0.0
25. Mem0 README (benchmark claims) — https://github.com/mem0ai/mem0/blob/f1791daa86d469249c9b002f62644d58a1eb96a0/README.md
26. Mem0 issues and PRs — #3004, #4896, #4923, #6515, #6531, #7123, #7879, PR #6530 — https://github.com/mem0ai/mem0/issues
27. Mem0 MCP docs — https://github.com/mem0ai/mem0/blob/f1791daa86d469249c9b002f62644d58a1eb96a0/docs/platform/mem0-mcp.mdx
28. mem0ai/openmemory — https://github.com/mem0ai/openmemory
29. mem0ai/memory-benchmarks — https://github.com/mem0ai/memory-benchmarks
30. Mem0 paper — https://arxiv.org/abs/2504.19413 (HTML v1)
31. Letta repo README — https://github.com/letta-ai/letta
32. Letta V1 archive (block ORM, history, constants, tools, settings) — https://github.com/letta-ai/letta/tree/56ba9c25552605eec89de8ed3dc6394b625c1993/letta
33. Letta Code repo and README — https://github.com/letta-ai/letta-code/blob/186d75b570aa4a58d94c182230dc179e0a9a29d0/README.md
34. Letta MemFS docs — https://docs.letta.com/concepts/memfs
35. Letta shared memory docs — https://docs.letta.com/concepts/shared-memory
36. Letta, "Context Repositories" — https://www.letta.com/blog/context-repositories
37. Letta, "Benchmarking AI Agent Memory: Is a Filesystem All You Need?" — https://www.letta.com/blog/benchmarking-ai-agent-memory
38. Letta, "Context-Bench" — https://www.letta.com/blog/context-bench
39. Sleep-time compute — https://arxiv.org/abs/2504.13171
40. MemGPT — https://arxiv.org/abs/2310.08560
41. LoCoMo paper — https://arxiv.org/abs/2402.17753
42. LoCoMo repo and issues #11, #27, #28, #35, #41, #43, #44, #45 — https://github.com/snap-research/locomo/issues
43. LongMemEval — https://arxiv.org/abs/2410.10813
44. MemoryAgentBench — https://arxiv.org/abs/2507.05257
45. Conflict-resolution reading of MemoryAgentBench — https://arxiv.org/abs/2610.09193
46. Scoring-target noninvariance — https://arxiv.org/abs/2605.24060
47. Retrieval vs utilisation bottlenecks — https://arxiv.org/abs/2603.02473
48. Fact memory vs long context, cost analysis — https://arxiv.org/abs/2603.04814
49. LTM in distributed multi-agent systems (mem0 / Graphiti / cognee) — https://arxiv.org/abs/2601.07978
50. Serving cost of agentic memory — https://arxiv.org/abs/2608.11879
51. MemoryTax — https://arxiv.org/abs/2610.29823
52. Storage Is Not Memory — https://arxiv.org/abs/2605.04897
53. VibeMemBench — https://arxiv.org/abs/2609.23570
54. MemDev-Bench (incl. Appendix F, Table 9) — https://arxiv.org/abs/2610.27524
55. LongMemEval-V2 — https://arxiv.org/abs/2605.12493
56. projectmem — https://arxiv.org/abs/2606.12329
57. MemMachine paper — https://arxiv.org/abs/2604.04853
58. A-MEM paper — https://arxiv.org/abs/2502.12110 ; repos https://github.com/agiresearch/A-mem , https://github.com/WujiangXu/A-mem , https://github.com/WujiangXu/A-mem-sys
59. Cognee — https://github.com/topoteretes/cognee (code at `8853378eae53`: cognify.py, `extract_content_graph.py`, `DataPoint.py`, graph/config.py); issues #5254, #6170, #6559
60. LangMem — https://github.com/langchain-ai/langmem (`8620b986af5b`: knowledge/extraction.py, reflection.py); issues #125, #189, #192, #196
61. LangGraph BaseStore and PostgresStore — https://github.com/langchain-ai/langgraph (`e5e37b5d70db`: libs/checkpoint/langgraph/store/base/__init__.py; checkpoint-postgres/…/store/postgres/base.py)
62. MemOS — https://github.com/MemTensor/MemOS (`3a16e5fe3048`: memories/textual/item.py, mem_reader/simple_struct.py, api/mcp_serve.py); arXiv 2505.22101, 2507.03724; issue #2409
63. Memobase — https://github.com/memodb-io/memobase (`358c16bbc6d6`: controllers/modal/chat/__init__.py, env.py, readme.md); issues #79, #101, #109
64. MIRIX — https://github.com/Mirix-AI/MIRIX (`98ebb81c7f45`: server/rest_api.py, functions/function_sets/memory_tools.py, orm/episodic_memory.py); arXiv 2507.07957; issue #6
65. Supermemory — https://github.com/supermemoryai/supermemory (`d3f1ee652505`: apps/docs/self-hosting/overview.mdx, concepts/graph-memory.mdx); issues #1299, #1627, #1826, #1827, #1890
66. Hindsight — https://github.com/vectorize-io/hindsight (`289fb421bdb4`: hindsight_api/config.py, engine/retain/fact_extraction.py, engine/consolidation/prompts.py, api/mcp.py); arXiv 2512.12818; issues #6241, #6335, #6471
67. Basic Memory — https://github.com/basicmachines-co/basic-memory (README, pyproject.toml, mcp/tools/edit.py, markdown/temporal_qualifier.py, hooks/); issues #1731, #1771
68. MCP reference memory server — https://github.com/modelcontextprotocol/servers/tree/main/src/memory (index.ts, README.md, CHANGELOG.md); issues #4797, #5058, #5485
69. txtai — https://github.com/neuml/txtai (agent/base.py, app/base.py, setup.py, docs/api/mcp.md)
70. LlamaIndex Memory — https://github.com/run-llama/llama_index/tree/main/llama-index-core/llama_index/core/memory ; issues #23090, #23806, #24117
71. CrewAI unified memory — https://github.com/crewAIInc/crewAI/tree/main/lib/crewai/src/crewai/memory ; docs/edge/en/concepts/memory.mdx; issues #5057, #5800, #6043
72. Microsoft Agent Framework — https://github.com/microsoft/agent-framework (python core `_sessions.py`, `_harness/_memory.py`; .NET `FileMemoryProvider.cs`, `Mem0Provider.cs`); issues #9723, #9997, #10127
73. Semantic Kernel — https://github.com/microsoft/semantic-kernel (README; `WhiteboardProvider.cs`; `semantic_text_memory.py`); Kernel Memory — https://github.com/dluc/kernel-memory
74. OpenAI Agents SDK sessions — https://github.com/openai/openai-agents-python/tree/main/src/agents/memory ; docs/sessions/index.md; issues #4775, #5507
75. OpenAI Codex memories — https://github.com/openai/codex/tree/main/codex-rs/memories ; https://learn.chatgpt.com/docs/customization/memories ; https://learn.chatgpt.com/docs/agent-configuration/agents-md ; issues #41711, #48381, #49347
76. Claude memory tool — https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool ; Claude Code memory — https://code.claude.com/docs/en/memory ; issues anthropics/claude-code #29178, #81833, #82056
77. Mastra memory and Observational Memory — https://github.com/mastra-ai/mastra/tree/main/packages/memory ; https://mastra.ai/research/observational-memory
78. Memori — https://github.com/MemoriLabs/Memori ; arXiv 2603.19935
79. EverOS — https://github.com/EverMind-AI/EverOS ; LongMemory — https://github.com/CaviraOSS/LongMemory ; memU — https://github.com/NevaMind-AI/memU ; MemMachine — https://github.com/MemMachine/MemMachine
80. ReasoningBank — https://arxiv.org/abs/2509.25140 ; https://github.com/google-research/reasoning-bank ; Acontext (archived) — https://github.com/memodb-io/Acontext ; LightMem — https://github.com/zjunlp/LightMem ; MemoryOS — https://github.com/BAI-LAB/MemoryOS ; Nemori — https://github.com/nemori-ai/nemori
