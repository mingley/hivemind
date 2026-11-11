# Research 07: Embeddings, rerankers, and the local-inference footprint

*Research thread for hivemind. All sources are public and were checked on 2026-11-10. Source numbers such as [S12] refer to §11.*

**Labels:** **[V]** checked against a primary source (code, model card, official docs or pricing page). **[C]** a claim made by a vendor or project about its own work, not reproduced here. **[I]** independent evidence (a third party's measurement or a peer-reviewed paper). **[Inf]** my inference or estimate. **[?]** unknown or unverified.

> **How to read the numbers.** I report benchmark scores as published, and they are often **not comparable** across rows. Three examples:
> - MTEB English v1 "Retrieval (15)" (the BEIR-15 subset) and MTEB English v2 "Retrieval" use different task sets.
> - CoIR averages depend on the protocol and the `mteb` version (§3.3).
> - Leaderboard scores are self-reported and may be inflated by overlap with training data. MTEB introduced RTEB, which uses private datasets, because of this generalization gap [S81][S80].
>
> I did not run any model. Every throughput figure for hivemind's workload (short memories) is an **estimate** with a stated derivation.

---

## 1. Summary

- **Ship lexical retrieval first, and always.** BM25 with a code-aware tokenizer should be both a **zero-model mode** and one half of every hybrid.
  - The BEIR authors call BM25 "a robust baseline" [S60] [I].
  - On CoIR (code retrieval), BM25 averages 42.31 nDCG@10, beating a 16M static code embedder alone (39.08). On CodeFeedback-MT it beats the 137M CodeRankEmbed (59.19 vs 42.61) [S34] [C].
  - Exact strings such as error codes defeat dense models. Anthropic's example is "TS-999" [S62] [C].
- **Recommended default for laptop mode:** BM25 plus a small transformer embedder through `fastembed-rs`/ONNX Runtime, fused with RRF.
  - Starting model: `bge-small-en-v1.5` int8 (33M parameters, 384 dims, 34 MB ONNX file, MIT). It is supported out of the box [S1][S47] [V].
  - Two challengers are worth an evaluation slot: **granite-embedding-small-english-r2** (47M, Apache-2.0, 8K context, CoIR 53.8 claimed) [S32] and **EmbeddingGemma 2 text-only** (270M, Apache-2.0, 8K, Matryoshka 768→128, MTEB-Code 78.68 claimed) [S20].
- **EmbeddingGemma 2 (released September 2026) is the most important new option** [S20][S21] [V/C].
  - It uses a permissive Apache-2.0 license, unlike EmbeddingGemma-300M, which uses the Gemma Terms of Use and is gated on Hugging Face [S47] [V].
  - The text-only footprint is 270M parameters. It claims about +10 MTEB-Code points over its predecessor and offers Matryoshka truncation.
  - It is **not yet in fastembed-rs 7.7.0** [S1] [V]. ONNX ports (q4 174 MB, int8 314 MB) and GGUF ports exist [S47] [V].
- **License traps.** The following are not suitable as defaults:
  - CC BY-NC 4.0: jina-embeddings-v3 and v5, jina-code-embeddings, and the jina-reranker v2/v3/v3.5 models.
  - jina-embeddings-v4: Qwen Research License [S26].
  - naver/splade-v3: CC BY-NC-SA.
  - EmbeddingGemma-300M: Gemma Terms of Use [S25–S29][S47] [V].
- **Rust runtime:** `fastembed-rs` is the pragmatic choice (Apache-2.0, v7.7.0, merging PRs weekly) [S1][S2][S3] [V].
  - It wraps `ort` 2.0.0-rc.15, which is still a release candidate and binds ONNX Runtime 1.31.
  - ort's prebuilt x86-64 binaries need an **x86-64-v3** CPU (AVX2-class).
  - On macOS, ort registers CoreML automatically unless told otherwise [S4][S1] [V].
  - Candle backs the Qwen3, nomic-v2-moe and Metal BERT paths. Keep `model2vec-rs` (MIT, pure Rust, no neural-network runtime) as the dependency-free fallback [S6] [V].
- **Static embeddings are fast but weak on natural-language retrieval.** On a desktop CPU, potion-base-8M runs about 124k sentences/s against 888/s for bge-small. However, its NanoBEIR score (0.442) is only on par with BM25 (0.452) and well below bge-small (0.627) [S67] [I]. Use static embeddings as a hybrid helper and fallback, not as the only semantic layer.
- **Code embedders:** for code-like memories, the best permissive small options are:
  - CodeRankEmbed (137M, MIT, CoIR about 59–60) [S36][S34] [C].
  - jina-embeddings-v2-base-code (161M, Apache-2.0, supported in fastembed-rs) [S1][S35] [V].
  - granite-small-r2 or EmbeddingGemma 2, which are general models with code training. These are probably the best compromise for **mixed** natural-language and code memories [Inf].
  - nomic-embed-code (7B) is too large for laptops [S38].
- **Rerankers: off by default on laptops.**
  - If enabled, use a 23 MB int8 `ms-marco-MiniLM-L6-v2` on the top 20 candidates. My estimate is tens of milliseconds on CPU [Inf].
  - In server mode, consider Apache-licensed `Qwen3-Reranker-0.6B`, `mxbai-rerank-base-v2` or `gte-reranker-modernbert-base`.
  - `bge-reranker-v2-m3` reportedly scores only 36.28 on CoIR [S49] [C].
- **Storage is not the bottleneck at 10k–1M memories.**
  - 1M × 384 dims costs 1.54 GB in f32, 384 MB in int8 and 48 MB in binary, plus about 128–160 MB of HNSW graph [S82] [V arithmetic].
  - How much quality quantization keeps depends on the model. With rescoring, binary vectors kept 96.45% for mxbai-large but only 74.77% for e5-base-v2 [S66] [C].
  - Matryoshka truncation to 256 dims keeps about 98–99% of the English MTEB score for EmbeddingGemma and nomic v1.5 [S18][S20][S16] [C].
- **Treat each combination of model, revision, quantization, runtime, pooling, prompt and dimension as a separate embedding space.** Tag every vector with its space ID, and migrate by dual-write plus background backfill (Qdrant 1.18 can now add or remove named vectors in place) [S68][S69] [V].
  - Estimated laptop-CPU time to re-embed 100k short memories: **3–10 min** with bge-small, **20–60 min** with EmbeddingGemma-2-text, **1–3 h** with Qwen3-Embedding-0.6B. For 1M memories, multiply by 10 [Inf; derivation in §8.4].
- **Hosted embedding cost is negligible at hivemind's scale.** OpenAI text-embedding-3-small and voyage-4-lite both cost $0.02 per million tokens, and gemini-embedding-001 costs $0.15/M [S75][S43][S77] [V]. Embedding 1M memories of about 64 tokens costs roughly $1–13. Privacy, offline use and latency dominate the decision, not price. Cohere no longer lists self-serve embedding prices [S79] [V].
- **We must build our own coding-memory evaluation.** Published retrieval numbers come from different benchmark versions and protocols, and none measures "short agent memories mixing natural language with identifiers and error strings" (§9).

---

## 2. Rust inference options

### 2.1 Runtime comparison

| Option | Version and license (crates.io / repo) | What you link and ship | Hardware | Models relevant to hivemind | Maturity |
|---|---|---|---|---|---|
| **fastembed-rs** (`fastembed`) | 7.7.0, Apache-2.0; 5.29M downloads [S3] [V] | `ort =2.0.0-rc.15` (ONNX Runtime 1.31) behind the `onnx` feature, plus `tokenizers` 0.23. Candle 0.11 is optional (`qwen3`, `nomic-v2-moe`, `bert-text`, `metal`, `cuda`). Default features include `image-models` and native-TLS hf-hub downloads [S1: Cargo.toml L37–86] [V]. | CPU via ORT. On macOS, ORT's default `MaxEfficiency` policy registers CoreML; `AutoDevicePolicy::PreferCPU` turns that off [S1: init.rs L54–60] [V]. Metal and CUDA are available only for candle-backed models [S1 README L380] [V]. | Dense ONNX models: bge (small/base/large) and their int8 variants, all-MiniLM-L6/L12, nomic v1/v1.5, mxbai-large, gte-base/large v1.5, snowflake-arctic v1 (xs–l), jina-v2-base-code/en, EmbeddingGemma-300M (fp32, Q, Q4), multilingual-e5, bge-m3. Candle models: Qwen3-Embedding 0.6B/4B/8B (including GGUF), nomic-v2-moe. Sparse: SPLADE++, bge-m3, opensearch doc-v3-gte. Rerankers: bge-reranker-base/v2-m3, jina-reranker-v1-turbo/v2 (ONNX), Qwen3-Reranker (candle) [S1 L22–96][S2] [V]. **Not supported:** EmbeddingGemma 2, arctic v2, granite r2, CodeRankEmbed, model2vec [V]. | Very active (about 30 PRs merged in October–November 2026) [S2] [V]. |
| **ort** | 2.0.0-rc.15 (still a release candidate), MIT OR Apache-2.0 [S3][S4] [V] | By default, downloads statically linked ONNX Runtime binaries built by pyke at build time. `load-dynamic` loads a shared library at runtime instead [S4 Cargo.toml L56, L71] [V]. Upstream ONNX Runtime 1.31.1 archives are 11.4 MB (linux-x64 .tgz) and 36.9 MB (osx-arm64 .tgz), compressed [S5] [V]. The size it adds to a linked binary is **[?]**; I estimate tens of MB [Inf]. | x86-64 prebuilt binaries require x86-64-v3. All macOS builds include CoreML. Other execution providers: CUDA/TensorRT, DirectML, WebGPU and others [S4 prebuilt-binaries L7–17] [V]. Alternative backends `ort-tract` and `ort-candle` are labelled "experimental" [S4 backends L19, L69–72] [V]. | Any ONNX graph | Widely used (25.7M downloads) [S3]. |
| **candle** | 0.11.1, MIT OR Apache-2.0 [S3] [V] | Pure Rust, with optional Accelerate, MKL, Metal and CUDA | CPU, Metal, CUDA | `candle-transformers` includes bert, jina_bert, nomic_bert, modernbert, xlm_roberta, gemma3, gemma4, qwen3, stella_en_v5 and nvembed_v2 [S8] [V]. Each model needs pooling glue written by hand [Inf]. | Mature, maintained by Hugging Face |
| **model2vec-rs** | 0.4.0, MIT [S6][S3] [V] | `tokenizers`, `safetensors` and `half`; hf-hub is optional. No neural-network runtime at all [S6 Cargo.toml L33–41] [V]. | CPU (rayon); wasm feature | Model2Vec static models (potion family), with f32/f16/i8 weights [S6] [V] | Official MinishLab port [V] |
| **tract** | 0.23.10 (`tract-onnx`), MIT OR Apache-2.0 [S3] [V] | Pure Rust; ONNX and NNEF | CPU SIMD kernels (x86, ARMv7/8, SVE), plus Metal and CUDA backends and wasm, per its README [S9] [V] | Generic ONNX models. Speed relative to ORT on BERT-style encoders is **[?]** | Mature (Sonos) |
| **burn** | 0.22.1 (0.23.0-pre.2), MIT OR Apache-2.0 [S3] [V] | A deep-learning framework. ONNX graphs are imported as generated Rust code through `burn-onnx` [S10] [V]. | CubeCL backends (CUDA, ROCm, Metal, Vulkan, WebGPU) and CPU [S10] [V] | Embedding models would need porting or import [Inf] | Active, but more of a training stack |
| **llama.cpp via `llama-cpp-2`** | 0.1.161, MIT OR Apache-2.0 [S3] [V] | Builds llama.cpp from C/C++ source (cmake). The crate ships `embeddings` and `reranker` examples [S11] [V]. | CPU, Metal, CUDA, Vulkan | GGUF ports exist for Qwen3-Embedding-0.6B (Q8 639 MB) and EmbeddingGemma 1 and 2 (EmbeddingGemma 2 Q8 310 MB) [S47] [V] | Pre-1.0 crate that tracks upstream closely |
| **EmbedAnything** (`embed_anything`) | 0.7.1 on crates.io (0.7.2 in the repo), Apache-2.0 [S3][S7] [V] | Heavy dependency set: candle 0.11, optional ort rc.10, model2vec-rs 0.2.1, tokio, reqwest, image, pdf2image, optional AWS SDK [S7 rust/Cargo.toml L20–99] [V] | CPU, Metal, CUDA | Dense, SPLADE, ColBERT (answerai-colbert-small, jina-colbert-v2), model2vec, Qwen3, EmbeddingGemma, rerankers [S7 README L177–187] [V] | It is an ingestion pipeline, not a lean library [Inf] |
| **TEI** (sidecar) | v1.9.4, Apache-2.0 [S13] [V] | A separate Rust server built on candle plus ONNX weight loading, serving HTTP and gRPC | CPU (x86, arm64), Metal on Mac, CUDA | Qwen3, Gemma3 (EmbeddingGemma), ModernBERT and voyage-4-nano embeddings, plus rerankers [S13] [V] | Production-grade, but it is a separate process |
| **Ollama** (sidecar) | v0.40.16, MIT [S12] [V] | Separate daemon. `POST /api/embed` accepts `dimensions` and `truncate` (default **true**) and returns `prompt_eval_count` [S12 api.md L1666–1712] [V]. | CPU, Metal, CUDA (llama.cpp-based) | Docs examples use `all-minilm`, `embeddinggemma` and `embeddinggemma2` [S12] [V] | Ubiquitous on developer laptops [Inf] |

### 2.2 What this means for hivemind [Inf]

- **Embed in-process with fastembed-rs and trim its features.** Use `default-features = false`, keep `onnx`, pick a rustls TLS option, and drop `image-models`. Use `ort-load-dynamic` if we prefer to ship or locate `libonnxruntime` separately [S1][S4].
  - fastembed's own README says "No dependency on Tokio" [S1].
  - Pin `ort` exactly, because it is an RC and fastembed pins `=2.0.0-rc.15`.
  - Force `PreferCPU` on macOS until CoreML is benchmarked, so that results are deterministic and comparable.
- **Keep a pure-Rust fallback path** with model2vec-rs plus BM25. It covers:
  - x86 machines without AVX2 (the ORT prebuilt-binary baseline);
  - musl and wasm targets;
  - "no native blobs" policies.
- **Offer Ollama and TEI as optional providers**, not as dependencies. Many developers already run Ollama, and its `/api/embed` exposes token counts and `dimensions`. **Never rely on `truncate: true` silently**: hivemind should count tokens with the model's own tokenizer and handle oversize memories explicitly.
- **Skip burn, tract and llama.cpp bindings for v1.** None has a ready-made embedding pipeline that beats fastembed-rs plus candle for our models. llama.cpp adds a C++ toolchain.

---

## 3. Embedding models

### 3.1 General-purpose and agent-relevant text embedders

Parameter counts come from the Hugging Face API safetensors totals [S47] unless noted. "Retrieval" means whatever retrieval metric the source published; read the benchmark column carefully. CPU throughput is the independent Hugging Face measurement: sentence-transformers on an i7-13700K desktop, embedding short GooAQ queries [S67] [I].

| Model | Params | Dims | Matryoshka (MRL) | Max tokens | License | Published retrieval-type score (source) | CPU throughput (source) | Supported in Rust today? |
|---|---|---|---|---|---|---|---|---|
| all-MiniLM-L6-v2 | 22.7M | 384 | no | 256 word pieces (truncates) [S14] | Apache-2.0 | MTEB Retrieval 41.66 [S66] or 42.92 [S33]; NanoBEIR 0.562 [S67] (all [C]/[I]) | 1,739 sent/s [I] | fastembed (fp32, Q) [V] |
| bge-small-en-v1.5 | 33.4M | 384 | no | 512 | MIT | MTEB-en v1 average 62.17, Retrieval(15) 51.68 [S15] [C]; NanoBEIR 0.627 [I] | 888 sent/s [I] | fastembed (default, plus Q) [V] |
| bge-base-en-v1.5 | 109.5M | 768 | no | 512 | MIT | Retrieval(15) 53.25 [S15] [C] | 265 sent/s [I] | fastembed [V] |
| snowflake-arctic-embed-xs / -s (v1) | 22M / 33M | 384 | no | 512 | Apache-2.0 | MTEB Retrieval 50.15 / 51.98 [S24] [C] | [?] | fastembed [V] |
| granite-embedding-small-english-r2 | 47.7M | 384 | not claimed | 8192 | Apache-2.0 | BEIR 50.9, MTEB-v2 61.1, CoIR 53.8 [S32] [C] | 199 docs/s on an H100 with 512-token chunks [C] | ONNX port (int8 52 MB) [S47]; candle `modernbert` [S8]; not in fastembed [V] |
| nomic-embed-text-v1.5 | 136.7M | 768 | 768→64 | 8192 | Apache-2.0 | MTEB average 62.28 at 768d [S16]; Retrieval 53.01 [S31] [C] | 86.9 sent/s [I] | fastembed (plus Q) [V] |
| nomic-embed-text-v2-moe | 475M total (305M active) | 768 | 768→256 | 512 | Apache-2.0 | BEIR 52.86 [S17] [C] | [?] | fastembed (candle feature) [V] |
| gte-modernbert-base | 149M | 768 | no | 8192 | Apache-2.0 | BEIR 55.33; "CoIR 79.31" on its own CoIR protocol (not comparable) [S31] [C] | [?] | candle `modernbert` [S8] [V] |
| mxbai-embed-large-v1 | 335M | 1024 | yes (and binary) | 512 | Apache-2.0 | Retrieval 54.39 [S66][S31] [C] | 79.8 sent/s [I] | fastembed [V] |
| snowflake-arctic-embed-m-v2.0 | 305M (113M non-embedding) | 768 | 256 | 8192 | Apache-2.0 | BEIR 55.4 [S23] [C] | [?] | not in fastembed (v1 only) [V] |
| EmbeddingGemma-300M | 303M (308M claimed) | 768 | 512, 256, 128 | 2048 | **Gemma Terms of Use**; gated on Hugging Face [S47] | MTEB-en v2 mean 69.67; MTEB-Code v1 68.76 [S18] [C] | "<15 ms for 256 tokens on EdgeTPU"; "<200 MB RAM with QAT" [S19] [C] | fastembed (fp32, Q, Q4) [V] |
| **EmbeddingGemma 2 (text only)** | 270M (130M transformer + 140M embedding table) of 740M total | 768 | 512, 256, 128 | 8192 | **Apache-2.0** [S20][S21] | MTEB-en v2 68.46; MMTEB v2 61.36; MTEB-Code v1 78.68 [S20] [C] | "~191 MB active RAM" for quantized text-only weights on a Pixel 11 Pro [S21] [C] | **not in fastembed 7.7.0**. ONNX: q4 174 MB, int8 314 MB, fp32 1.08 GB. GGUF: Q8 310 MB [S47]. Ollama `embeddinggemma2` [S12] [V] |
| Qwen3-Embedding-0.6B | 596M | 1024 | 32–1024 | 32K | Apache-2.0 | MTEB-en v2 mean 70.70, Retrieval 61.83; MMTEB 64.33 [S22] [C] | [?] | fastembed (candle `qwen3`, GGUF) [S1]; TEI [S13] [V] |
| voyage-4-nano | 340M (180M non-embedding + 160M embedding) | 2048, 1024, 512, 256 | yes | 32K | Apache-2.0 [S41] | Results published only in a spreadsheet [?]. Shares an embedding space with the hosted voyage-4 models [S40] [C] | [?] | TEI [S13] [V] |
| jina-embeddings-v3 | 572M | 1024 | yes | 8192 | **CC BY-NC 4.0** | NanoBEIR 0.617 [S67] [I] | 0.55 sent/s (likely a pathological setup) [S67] [I] | no |
| jina-embeddings-v4 | 3.75B | 2048 | yes | 32768 | **Qwen Research License** ("initially released under cc-by-nc-4.0 due to an error") [S26] | — | — | no |
| jina-embeddings-v5-text-nano / -small | 239M / 677M | 768 / 1024 | yes | 8192 / 32768 | **CC BY-NC 4.0** | MTEB-en v2 71.0 / 71.7 [S27][S28] [C] | — | no |
| hs-embed-1 (Hindsight) | 112M (13M transformer + 98M embedding table) | 384→64 | yes | 8192 | MIT | Published only as images [?] | int8 ONNX for CPU [S46] [C] | ONNX 152 MB [S47] [V] |
| potion-base-8M (static) | 7.6M | 256 | — | unbounded | MIT | NanoBEIR 0.442 (BM25 scores 0.452) [S67] [I] | 124k sent/s (Python) [I]; Rust 2.60M texts/s on all cores of an M3 Pro [S6] [C] | model2vec-rs [V] |
| potion-retrieval-32M (static) | 32.3M | 512 | — | unbounded | MIT | MTEB Retrieval 35.06 (81.7% of MiniLM) [S33] [C] | same order as potion-base-8M [Inf] | model2vec-rs [V] |

**Newer models seen but not evaluated in depth:** nvidia Nemotron-3-Embed-1B (license tag `openmdw-1.1`), perplexity pplx-embed-v1-0.6b (MIT; native int8/binary output), LiquidAI LFM2.5-Embedding-350M (license `lfm1.0`), and ibm granite-embedding-97m/311m-multilingual-r2 (Apache-2.0) [S47] [V metadata only].

### 3.2 Code-specific embedders and code benchmarks

| Model | Params | Dims | Max tokens | License | Code-retrieval claims | Rust support |
|---|---|---|---|---|---|---|
| jina-embeddings-v2-base-code | 161M | 768 | 8192 | Apache-2.0 | CoIR 58.4 and CodeSearchNet MRR 67.2, as reported by the CodeRankEmbed authors [S36] [C] | fastembed [V] |
| CodeRankEmbed | 137M | 768 | 8192 | MIT | CoIR 60.1 (own) / 59.14 (MinishLab, mteb≥2.10); CodeSearchNet MRR 77.9. A query prefix is required [S36][S34] [C] | Not in fastembed. It uses the NomicBERT architecture, which candle's `nomic_bert` covers [S8] [Inf] |
| nomic-embed-code | 7.07B | — | — | Apache-2.0 | CodeSearchNet: 81.7 (Python) … 93.8 (Go), close to voyage-code-3 [S38] [C] | Too large for laptops [Inf] |
| jina-code-embeddings-0.5b | 494M | 896→64 | 32768 | **CC BY-NC 4.0** | CoIR 73.94, as reported in the Jina reranker card [S49] [C] | no |
| potion-code-16M-v2 (static, distilled from CodeRankEmbed) | 16.2M | 256 | unbounded | MIT | CoIR: 39.08 alone; **43.36 in an RRF hybrid with BM25**; BM25 alone 42.31; CodeRankEmbed 59.14 [S34] [C] | model2vec-rs [V] |
| granite-embedding-small-english-r2 | 47.7M | 384 | 8192 | Apache-2.0 | CoIR 53.8 (bge-small scores 45.8 in IBM's table) [S32] [C] | ONNX / candle [V] |
| EmbeddingGemma 2 | 270M (text) | 768 | 8192 | Apache-2.0 | MTEB-Code v1: 78.68 at 768d, 76.18 at 256d [S20] [C] | ONNX / GGUF ports [V] |
| LateOn-Code-edge / LateOn-Code (ColBERT) | 16.8M / 149M | multi-vector | — | Apache-2.0 | NDCG@10 0.846 / 0.859 on the Semble code-search benchmark (1,251 queries, 63 repos); Semble with potion-code scores 0.853 [S57] [C] | ColGREP is a single Rust binary built on ONNX Runtime [S57] [V] |
| voyage-code-3 / voyage-code-4 (API) | — | 2048, 1024, 512, 256 | 32K | proprietary | voyage-code-3: "+13.80% over OpenAI-v3-large on 32 code retrieval datasets" [S39] [C]. voyage-code-4 is "optimized for code retrieval and coding agent use cases" [S42] [V] | HTTP only |

### 3.3 Caveats that change decisions

- **CoIR is not one number.** CodeRankEmbed reports 60.1 itself, while MinishLab measured 59.14 with mteb≥2.10 [S36][S34]. gte-modernbert-base reports 79.31 with a different aggregation [S31]. Jina's reranker table puts bge-reranker-v2-m3 at 36.28 [S49]. CoIR's own paper stresses that even state-of-the-art systems struggle with code retrieval [S45] [I]. Compare only numbers from the same harness.
- **Most code benchmarks are natural language → code, or code → code.** Hivemind's memories are mostly *natural-language claims containing code tokens*, such as error text, identifiers and versions. The closest public analogues are CodeFeedback and StackOverflow-QA inside CoIR, where BM25 is strong: 59.19 and 70.26 nDCG@10 against CodeRankEmbed's 42.61 and 80.53 [S34] [C]. This is why the hybrid design matters more than which code embedder we pick [Inf].
- **Older models score higher on BEIR-style benchmarks partly because of training-set overlap.** The Hugging Face static-embedding post warns that "many of the attention-based dense embedding models are finetuned on the training splits of the (Nano)BEIR evaluation datasets" [S67] [I]. RTEB exists for this reason [S81].

---

## 4. Footprint: what has been measured and what is estimated

### 4.1 Measured or claimed

- **Independent CPU throughput** [S67] [I], measured with sentence-transformers on an i7-13700K embedding short queries:

| Model | Sentences/s |
|---|---|
| potion-base-8M | 124,030 |
| BM25 scoring | 49,707 |
| all-MiniLM-L6-v2 | 1,739 |
| mxbai-embed-xsmall-v1 | 1,749 |
| bge-small | 888 |
| bge-base | 265 |
| gte-base-v1.5 | 198 |
| nomic v1.5 | 87 |
| mxbai-large | 80 |

- **model2vec-rs** [S6] [C], on an Apple M3 Pro, embedding 666,808 Wikipedia lines that average 104 bytes:
  - potion-base-8M: 575K texts/s on one thread and 2.60M texts/s on all cores, with a 24 ms model load.
  - Its int8 mode keeps per-row scales, and its outputs have cosine similarity above 0.9998 to f32 [S6 L128] [C].
- **Model file sizes** [S47] [V]:

| Model | Files |
|---|---|
| bge-small ONNX | 133 MB fp32, 34 MB int8 |
| all-MiniLM-L6-v2 ONNX | 90 MB |
| ms-marco-MiniLM-L6-v2 ONNX | 91 MB fp32, 23 MB int8 |
| potion-base-8M | 30 MB |
| potion-code-16M-v2 | 32 MB (f16) |
| granite-small-r2 ONNX | 195 MB fp32, 52 MB int8 |
| nomic v1.5 ONNX | 547 MB fp32, 137 MB int8 |
| EmbeddingGemma-300M ONNX | 1.24 GB fp32, 309 MB int8, 197 MB q4 |
| EmbeddingGemma 2 text ONNX | 1.08 GB fp32, 314 MB int8, 174 MB q4 |
| Qwen3-Embedding-0.6B GGUF | 1.2 GB f16, 639 MB Q8 |

### 4.2 Estimates for hivemind memories [Inf]

Assume about 64 tokens per memory and a laptop CPU with 8–12 cores. The anchors come from §4.1.

- **Length correction.** Short GooAQ queries are about 12–16 tokens, so I scale throughput by about 0.25 for 64-token memories.
- **Runtime correction.** ONNX Runtime int8 might be 1–2× faster than PyTorch fp32 [?].
- **Model-size correction.** Larger models are scaled from bge-small by their ratio of non-embedding parameters:
  - bge-small: about 21M;
  - EmbeddingGemma-2 text: about 130M (6×);
  - Qwen3-0.6B: about 440M (about 20×).
- **Sanity check.** Parameter-ratio scaling predicts bge-base at 4× slower than bge-small, and the measurement shows 3.35×. nomic v1.5, however, ran about 2× slower than its parameter count predicts. Treat every estimate as ±2×.

| Setup | CPU time per 1k memories (wall time, multi-threaded) | Embed one query (single short query) | Resident model RAM (excluding vectors) |
|---|---|---|---|
| BM25 only (tantivy or FTS5) | ≪0.1 s to index | — | Index only; tantivy claims under 10 ms startup [S83] |
| potion-base-8M / potion-code-16M (model2vec-rs) | about 1–5 ms | under 1 ms | about 30 MB (about 8 MB int8) |
| all-MiniLM-L6-v2 (ORT int8) | about 1–2.5 s | about 2–5 ms | about 25 MB of weights plus ORT runtime; total RSS 80–200 MB [?] |
| bge-small-en-v1.5 (ORT int8) | about 2–5 s | about 3–10 ms | about 35 MB of weights plus ORT; total RSS 100–250 MB [?] |
| EmbeddingGemma 2 text (ORT int8 or q4) | about 15–30 s | about 15–40 ms | 174–314 MB of weights; Google claims about 191 MB active on a phone [S21] |
| Qwen3-Embedding-0.6B (candle CPU, f32 or Q8) | about 45–90 s | about 50–150 ms | 0.64–2.4 GB |

---

## 5. Rerankers and late interaction

| Model | Params | License | Claimed quality | Claimed speed | Rust path |
|---|---|---|---|---|---|
| cross-encoder/ms-marco-MiniLM-L6-v2 | 22.7M | Apache-2.0 | TREC-DL19 nDCG@10 74.30; MS MARCO MRR@10 39.01 [S52] [C] | 1,800 docs/s on a V100 GPU [S52] [C] | ONNX int8, 23 MB [S47]. Usable through fastembed's user-defined models; it is not in fastembed's built-in list [S1] [Inf] |
| mxbai-rerank-xsmall-v1 | 70.8M | Apache-2.0 | [?] | [?] | ONNX [Inf] |
| gte-reranker-modernbert-base | 149.6M | Apache-2.0 | BEIR 56.19; "CoIR 79.99" on gte's own protocol [S31] [C] | [?] | ModernBERT via ONNX or candle [Inf] |
| granite-embedding-reranker-english-r2 | 149.6M | Apache-2.0 | [?] | [?] | [?] |
| hs-rerank-1 (Hindsight) | 114M (15M transformer) | MIT | Published only as images [?] | int8 ONNX for CPU [S46] [C] | ONNX [V] |
| bge-reranker-v2-m3 | 568M | Apache-2.0 | BEIR 56.51; **CoIR 36.28** in Jina's table [S49] [C] | [?] | fastembed [V] |
| mxbai-rerank-base-v2 | 494M | Apache-2.0 | BEIR 55.57 (own), 58.40 (Jina v3 table) or 59.58 (Jina v3.5 table); CoIR 65.71 (Jina) [S51][S49][S50] [C] | 0.67 s on an A100 for its benchmark setting [S51] [C] | candle (Qwen2-based) [Inf] |
| Qwen3-Reranker-0.6B | 596M | Apache-2.0 | BEIR 56.28 and CoIR 65.18 in Jina's table [S49] [C] | [?] | fastembed candle (PR #331) [S2] [V] |
| jina-reranker-v3 / v3.5 | 0.6B | **CC BY-NC 4.0** | BEIR 61.94 / 63.20 [S49][S50] [C] | — | — |
| answerai-colbert-small-v1 (late interaction) | 33M; 96 dims per token | Apache-2.0 | BEIR average 53.79 (bge-small: 51.68) [S55] [C] | [?] | EmbedAnything [S7]; NextPlaid (Rust, Apache-2.0) [S57] [V] |
| LateOn-Code-edge (ColBERT) | 16.8M | Apache-2.0 | See §3.2 | "runs on CPU" [S57] [C] | ColGREP / NextPlaid [V] |

The reranker scores above are inconsistent even within one vendor's materials. Jina's v3 and v3.5 tables give different BEIR numbers for the same baselines, so evaluation setup dominates [S49][S50] [C].

**CPU cost per query at small k** [Inf]:

- A cross-encoder runs one forward pass per (query, memory) pair. With a 16-token query and a 64-token memory, that is about 80 tokens per pair.
  - MiniLM-L6 at k=20 means 20 short forward passes, roughly 20–60 ms on a laptop CPU.
  - A 150M ModernBERT reranker takes roughly 150–400 ms.
  - A 0.6B reranker (Qwen3 or mxbai-base-v2) takes about 0.5–2 s on CPU, which is acceptable only in server mode, on a GPU or Metal, or asynchronously.
- **Late interaction (ColBERT)** moves the cost to storage. With answerai-colbert-small, a 64-token memory has 64 × 96 dimensions:
  - about 12 KB per memory in f16, or about 12 GB per 1M memories;
  - about 1.6–2 GB per 1M with roughly 2-bit residual compression.
  - The query costs one 33M-parameter encoding plus MaxSim over the candidates.
- Late interaction looks attractive for code (ColGREP, LateOn-Code) [S57]. For short natural-language memories, a single vector plus BM25 is simpler. Revisit if the evaluation shows a gap.

**Prior art built specifically for agent memory.** Hindsight (vectorize-io; MIT; about 60k GitHub stars) trains its own small embedder and reranker (hs-embed-1, hs-rerank-1) for agent memory and also runs a BM25 path [S46] [V]. This suggests memory-specific fine-tuning of tiny models is a viable later step for hivemind [Inf].

---

## 6. Lexical, sparse and hybrid retrieval

**BM25 evidence**

- BEIR: "BM25 is a robust baseline". Reranking and late interaction score best, but "at high computational costs" [S60] [I].
- NanoBEIR, measured by Hugging Face: BM25 0.452, potion-base-8M 0.442, static-retrieval-mrl-en-v1 0.503, MiniLM-L6 0.562, bge-small 0.627 [S67] [I].
- CoIR (code), measured by MinishLab: BM25 average 42.31. It is strongest on CodeFeedback-MT (59.19, beating CodeRankEmbed's 42.61) and StackOverflow (70.26) [S34] [C].
- Exact identifiers fail with dense models. Anthropic's example is the query "Error code TS-999" [S62] [C].
- Rust implementations:
  - `tantivy` 0.27 (MIT): BM25 "the same as Lucene", configurable tokenizers, mmap, under 10 ms startup [S83][S3] [V].
  - SQLite FTS5 provides a `bm25()` ranking function [S84] [V].
  - The `bm25` crate (MIT) also emits sparse vectors for vector stores [S85] [V].

**Code-aware tokenization is the high-leverage piece** [Inf]:

- Keep the raw token: `E0277`, `v1.82.0`, `aarch64-apple-darwin`, `src/foo/bar.rs`.
- Also emit split sub-tokens: `camelCase` → `camel` and `case`; `snake_case` → `snake` and `case`; paths split on `/` and `.`.
- Optionally index character n-grams for partial identifiers.
- Semble does this in its ranking signals: "symbol-like queries … get more lexical weight", plus identifier-stem matching [S59] [C].

**Learned sparse retrieval**

- SPLADE expands queries and documents into weighted vocabulary terms [S63] [I]. fastembed-rs supports SPLADE++ (`prithivida/Splade_PP_en_v1`, Apache-2.0), the sparse output of bge-m3, and `opensearch-neural-sparse-encoding-doc-v3-gte` [S1] [V].
- The opensearch model is "inference-free" at query time: the query side needs only the tokenizer and an IDF table. It claims an average nDCG@10 of 0.546 on 13 BEIR datasets [S65] [C].
- `naver/splade-v3` is CC BY-NC-SA [S47] [V].
- Learned sparse retrieval behaves well on English prose, but its behaviour on identifiers and code tokens is **[?]**. It belongs in the evaluation, not the default.

**Fusion evidence**

- Anthropic: embeddings plus BM25 beat embeddings alone. Contextual embeddings plus contextual BM25 cut top-20 retrieval failures by 49%, and adding a reranker cut them by 67% (from 5.7% to 1.9%) [S62] [C].
- Bruch et al. (ACM TOIS) found that a tuned convex combination of normalized scores outperforms RRF in-domain and out-of-domain, that RRF "is sensitive to its parameters", and that the convex combination needs only a few labelled examples [S61] [I].
- potion-code plus BM25 with RRF (k=60) beats either retriever alone on CoIR (43.36 against 39.08 and 42.31) [S34] [C].
- Semble fuses static embeddings with BM25 plus code-aware signals and reports NDCG@10 of 0.855, "matching CodeRankEmbed while indexing 480× faster" [S59] [C].

**Implication** [Inf]: start with RRF because it needs no calibration and the score scales differ. Log query/result features so that we can fit a convex combination, or an adaptive lexical weight for symbol-like queries, once we have labels.

---

## 7. Quantization and compression

### 7.1 Raw vector storage per 1M vectors

Exact arithmetic [V]: f32 uses 4 bytes per dimension, int8 1 byte, 4-bit 0.5 bytes, and binary 1 bit.

| Dims | f32 | f16 | int8 | 4-bit (TurboQuant TQ4 / PQ) | binary |
|---|---|---|---|---|---|
| 1024 | 4.10 GB | 2.05 GB | 1.02 GB | 512 MB | 128 MB |
| 768 | 3.07 GB | 1.54 GB | 768 MB | 384 MB | 96 MB |
| 512 | 2.05 GB | 1.02 GB | 512 MB | 256 MB | 64 MB |
| 384 | 1.54 GB | 768 MB | 384 MB | 192 MB | 48 MB |
| 256 | 1.02 GB | 512 MB | 256 MB | 128 MB | 32 MB |
| 128 | 512 MB | 256 MB | 128 MB | 64 MB | 16 MB |

- An HNSW graph adds about M × 8–10 bytes per vector, so about 128–160 MB per 1M vectors at M=16 [S82] [V].
- At 100k memories, divide everything by 10. Even f32 at 768 dims is about 307 MB, which is fine on a laptop.

### 7.2 Quality retained

| Technique | Evidence |
|---|---|
| **int8 (scalar), no rescoring** | Share of the f32 retrieval score kept: mxbai-large 97%, e5-base-v2 94.68%, MiniLM-L6 90.79% [S66] [C] |
| **int8 with rescoring** (rescore multiplier 4–5) | About 99% retained [S66] [C] |
| **Binary with f32-query rescoring** | mxbai-large 96.45%, MiniLM-L6 93.79%, nomic v1.5 87.7%, e5-base-v2 74.77%. Strongly model-dependent. Speedup up to about 25× on average [S66] [C] |
| **Matryoshka truncation** (computed from cards) [C] | EmbeddingGemma 2 at 256d: 99.0% of MTEB-en v2 and 96.8% of MTEB-Code. At 128d: 95.9% and 90.8% [S20]. EmbeddingGemma-300M at 256d: 98.1% of MTEB-en v2 [S18]. nomic v1.5 at 256d: 98.0%; at 128d: 95.3% [S16]. arctic-m-v2 at 256d: "about 3% degradation" [S23] |
| **Quantization-aware APIs** | voyage-4 and voyage-code-3 return int8, uint8 and binary vectors natively [S39][S40]. pplx-embed outputs int8 natively [S47] [C] |
| **Qdrant TurboQuant (v1.18)** | 4-bit TQ4 recall is close to scalar quantization at twice the compression: 0.9169–0.9271 against SQ's 0.9014–0.9339. 1-bit TQ1 beats 1-bit BQ1 recall (for example 0.7924 vs 0.6921) but is slower [S71] [C] |

**Implication** [Inf]:

- **Laptop scale (≤1M memories):** store f32, or int8 with an f32 rescore over the top 50. Binary buys nothing that matters at this size and risks silent quality loss on models that weren't trained for it.
- Truncating to 256 dims with an MRL model is the cleaner lever. Store the full vector and index the truncated, re-normalized prefix. Never mix dimensions; the EmbeddingGemma 2 card warns that skipping re-normalization "degrades ranking quality silently" [S20] [V].

---

## 8. Embedding model changes

### 8.1 What counts as a different embedding space [Inf, grounded in V]

Vectors are only comparable if *all* of these match:

- model repo and **revision SHA**;
- weight quantization (fp32, int8 or q4 ONNX, GGUF Q8);
- runtime and kernel (ORT, candle, Metal);
- pooling and normalization;
- **prompt templates and prefixes** (EmbeddingGemma's `task: search result | query:`, CodeRankEmbed's required query prefix, nomic's `search_document:`) [S20][S36][S16];
- max length and truncation policy;
- output dimension, and whether it was truncated by MRL.

**Concrete warning [V]:** model2vec-rs's int8 mode (one scale per row) and Python model2vec's int8 mode (one scale per table) produce embeddings whose cosine similarity can drop to **0.27** [S6 L245]. Same model and nominal "int8", but an incompatible space.

### 8.2 How others handle it

- **Qdrant:**
  - Named vectors hold one vector field per model in one collection.
  - Since **v1.18**, named vectors "can be added to or removed from an existing collection without having to recreate the collection", explicitly for embedding-model migration [S69][S71] [V].
  - The official migration tutorial documents two approaches [S68] [V]:
    - **blue-green**: two collections with dual writes, a background re-embedding scroll, and an alias switch;
    - **named vectors**: add a vector field, dual-write, backfill, switch, then delete the old field. Rollback stays trivial.
  - v1.16 added **conditional updates** (optimistic concurrency on a version field) to resolve races between application writes and background re-embedding [S70] [V].
- **LanceDB:**
  - An embedding-function registry binds a source column to a vector column, and the binding is persisted in Arrow schema metadata under `embedding_functions`.
  - Multiple embedding definitions are allowed.
  - Query vectors are recomputed from that metadata. Data that is ingested with the vector column present (non-null) is not re-embedded [S72] [V].
  - Re-embedding means adding a new vector column (`add_columns`) and backfilling it [S72] [V].
- **Voyage 4: shared embedding space.** voyage-4-large, voyage-4, voyage-4-lite and the open-weight voyage-4-nano "produce compatible embeddings". Documents can be embedded with a large model and queries with a small one, and the query model can be upgraded "without re-vectorizing documents" [S40][S42] [C]. This is attractive but ties us to one vendor family.
- **Adapters instead of re-embedding:**
  - Drift-Adapter learns a mapping from new-model query vectors into the old space (Procrustes, low-rank affine or a small MLP). It reports recovering 95–99% of full re-embedding recall [S73] [C/I].
  - Backward-compatible embedding training is an older research line [S74] [I].
  - For hivemind, re-embedding is cheap enough (§8.4) that adapters are a later optimization [Inf].

### 8.3 Recommended policy for hivemind [Inf]

1. **The canonical store is text, not vectors.** Memories are short, so re-embedding is always possible. Vectors are a derived and disposable index.
2. Keep an `embedding_spaces` table with `space_id = hash(model_id, revision_sha, weights_digest, quantization, runtime+version, pooling, normalize, query/doc prompt templates, max_len, native_dim)`. Each vector row carries `space_id` and its stored dimension. **Never score across spaces.**
3. **One index per space.** Named-vector style is cleanest; a separate HNSW per `(space_id, dim)` also works.
4. **Migration:**
   - register the new space;
   - dual-write new memories;
   - backfill in the background, ordered by recency and access frequency, throttled to idle CPU on laptops;
   - shadow-evaluate on the hivemind evaluation set plus a sample of logged queries (recall overlap and nDCG);
   - flip reads;
   - keep the old space for N days, then drop it.
5. **During partial backfill,** query every active space plus BM25 and fuse **by rank (RRF)**, not by score, because cosine scales differ across models. BM25 is unaffected by model changes, which is another reason it must always be on.
6. **Shared or server mode:**
   - The safest contract is "clients send text and the server embeds", which keeps one authoritative space.
   - If clients embed locally (laptop offload), the client must declare its `space_id`, and the server rejects vectors whose space it doesn't serve.
   - Use conditional or versioned updates, as in Qdrant 1.16, so that a backfill never overwrites a newer edit.

### 8.4 Estimated time to re-embed short memories (about 64 tokens each) [Inf]

This uses the method in §4.2: Hugging Face's measured CPU throughput × 0.25 for length × 1–2 for ORT int8, scaled by non-embedding parameters. Expect ±2×. A GPU, Metal or a fast desktop would be several times faster [?].

| Model (laptop CPU, 8–12 cores) | 100k memories | 1M memories | Basis |
|---|---|---|---|
| potion-base-8M / potion-code-16M (model2vec-rs) | under 1 s | about 1–5 s | 2.6M texts/s on all cores of an M3 Pro for about 25-token texts [S6] [C] |
| all-MiniLM-L6-v2 (int8) | about 2–5 min | about 20–50 min | 1,739/s on short queries [S67] [I] |
| bge-small-en-v1.5 (int8) | about 3–10 min | about 0.5–1.5 h | 888/s on short queries [S67] [I] |
| EmbeddingGemma 2 text (int8 or q4) | about 20–60 min | about 3–10 h | about 6× the non-embedding compute of bge-small [S20] [Inf] |
| Qwen3-Embedding-0.6B (CPU) | about 1–3 h | about 10–30 h | about 20× the non-embedding compute of bge-small [S22] [Inf] |
| Hosted API (OpenAI, Voyage, Gemini) | minutes, rate-limited | under 1 h, rate-limited | $1–13 for 64M tokens (§10) [V prices] |

---

## 9. Experiments needed to choose

| ID | Question | Design | Decision rule (proposed) |
|---|---|---|---|
| **E1** | Retrieval quality on *coding memories* | Build a labelled set of about 2–5k queries over about 50–100k memories from public data: GitHub issue → fix pairs, StackOverflow Q/A, CI error logs with their resolutions, public agent trajectories (for example SWE-bench), and CoIR CodeFeedback/StackOverflow as a sanity check. Query types: natural language, pasted error string or stack-trace line, identifier or path, version-specific. Report Recall@5/10, nDCG@10 and MRR **per query type**. | Pick the cheapest configuration within 1–2 nDCG points of the best. |
| **E2** | Lexical tokenizer variants | BM25 with a default analyzer, then with code-aware splitting and raw tokens kept, then with n-grams; tantivy and FTS5. | Fix the default tokenizer before any dense experiments. |
| **E3** | Dense model shoot-out | potion-retrieval-32M and potion-code-16M-v2; MiniLM-L6; bge-small (int8); granite-small-r2; EmbeddingGemma 2 text at 768 and 256 dims; Qwen3-0.6B at 1024 and 256 dims; CodeRankEmbed; jina-v2-base-code; voyage-4-lite or nano as the hosted reference. | Choose the laptop default and the server default. |
| **E4** | Fusion | Each dense model alone; BM25 + dense with RRF (k=60 and swept); a tuned convex combination [S61]; an adaptive lexical weight for symbol-like queries [S59]. | Ship RRF unless the convex combination wins by more than 2 points robustly. |
| **E5** | Rerankers | Top 20 / top 50 from the best hybrid, reranked by ms-marco-MiniLM-L6 (int8), gte-reranker-modernbert-base, granite-reranker-r2, hs-rerank-1, Qwen3-Reranker-0.6B, mxbai-rerank-base-v2. Measure p50 and p95 latency on a laptop CPU and on Metal. | Enable a reranker by default only if it adds at least 3 nDCG points at p95 under 100 ms on a laptop. |
| **E6** | Compression | f32, int8, int8 with rescore, binary with rescore, and MRL 256/128, for the top two models. | Choose the storage default per mode. |
| **E7** | Footprint and latency | On an M-series Mac (M1/M3) and an x86 laptop (one AVX2 machine and one without, to test the ORT x86-64-v3 baseline): binary-size delta from fastembed/ort/candle; cold start; RSS; ingest throughput for 1k, 100k and 1M memories; single-query latency; CPU vs CoreML vs Metal; ORT vs candle vs tract numerical parity (cosine similarity ≥ 0.999). | Validate or replace the estimates in §4.2 and §8.4. |
| **E8** | End-to-end agent outcome | Agents with and without hivemind retrieval under each candidate setup. Measure task success, LLM tokens consumed and wall time per task. | The final arbiter of "better outcomes per compute and token". |
| **E9** | Migration drill | Switch from model A to model B on 1M memories while serving queries; measure backfill time, quality during the dual-space RRF window, and correctness under concurrent edits. | Validates §8.3. |

---

## 10. Hosted embedding API pricing and token accounting

| Provider and model | Price per 1M input tokens | Batch price | Free allowance | Max input | Dims | How tokens are counted |
|---|---|---|---|---|---|---|
| OpenAI text-embedding-3-small | **$0.02** [S75] [V] | (not checked) | — | 8192 [S76] [V] | 1536 (configurable through `dimensions`) [S76] | Billed per input token. Count with `tiktoken` `cl100k_base` [S76] [V]. |
| OpenAI text-embedding-3-large | **$0.13** [S75] [V] | — | — | 8192 | 3072 (configurable) | same |
| Voyage voyage-4-lite / voyage-4 / voyage-4-large | **$0.02 / $0.06 / $0.12** [S43] [V] | 33% discount (Batch API) [S43] | First 200M tokens free for the v4 family [S43] [V] | 32K [S42] | 2048, 1024, 512, 256 | Per token, using tokenizers published on Hugging Face (`voyageai/voyage-4-large`) [S44] [V] |
| Voyage voyage-code-4 / voyage-code-3 | **$0.12 / $0.18** [S43] [V] | 33% discount | The page says voyage-code-3 gets 200M free tokens but also lists it among "older models" with no free tokens. This is **inconsistent [?]** | 32K | 2048, 1024, 512, 256 | same |
| Voyage rerank-3 / rerank-3-lite | **$0.05 / $0.02** [S43] [V] | — | 200M tokens for rerank-2.5 and 2.5-lite [S43] | — | — | Tokens = query tokens × number of documents + the sum of document tokens [S43] [V] |
| Google gemini-embedding-001 | **$0.15** [S77] [V] | $0.075 | Free tier exists, but its content is used to improve Google's products [S77] | 2,048 [S78] [V] | 128–3072 (768, 1536 or 3072 recommended) [S78] | Per input token |
| Google gemini-embedding-2 (multimodal) | **$0.20** for text; images $0.45/M tokens [S77] [V] | $0.10 for text | Free tier, same terms | [?] | [?] | Per token; media is billed per image, second or frame [S77] |
| Cohere Embed (v4 / "Embed 5") | **Not publicly listed** as of 2026-11-10. The pricing page shows only Model Vault GPU-hour prices and "contact sales" [S79] [V] | — | Trial keys are free but rate-limited and not for production use [S79] | [?] | [?] | Per token embedded [S79] |

**Scale check** [Inf]:

- 1M memories × 64 tokens = 64M tokens. That costs about $1.28 with OpenAI 3-small or voyage-4-lite ($0 inside Voyage's free 200M), $3.84 with voyage-4, $7.68 with voyage-code-4, $8.32 with OpenAI 3-large, $9.60 with gemini-embedding-001, and $12.80 with gemini-embedding-2.
- 10k queries per day × 20 tokens comes to well under $0.01 per day.
- Embedding spend is therefore **not** a meaningful part of hivemind's token economics. The relevant "tokens" are **LLM context tokens saved by better retrieval**, which is where E8 measures value.

**Token accounting notes for hivemind** [Inf]:

- Count tokens with the *active* embedding model's tokenizer when deciding truncation, and record that count per memory.
- Code-like text (hashes, paths, hex, stack traces) tokenizes much less efficiently than prose, so budget by tokens, not characters.
- Ollama's `/api/embed` returns `prompt_eval_count` and truncates by default [S12] [V]. Disable silent truncation and handle oversize memories by chunking or summarizing.

---

## 11. Recommendation

### 11.1 Laptop mode (single developer, local, minimal resources)

**Tier L0 — lexical only. Always present, and the fallback when no model is available.** Ship it: **yes.**
- **Stack:** BM25 with a code-aware analyzer, using tantivy (or SQLite FTS5 if the store is SQLite). It keeps raw tokens and adds sub-tokens split on case, underscores, paths and dots.
- **Footprint:** tens of MB of index per 1M short memories [?]; startup under 10 ms (claimed [S83]).
- **Why:** it is strong on error strings and identifiers [S62][S34], needs no download, and is unaffected by model changes. It is also the first-run experience while a model downloads.

**Tier L1 — default hybrid**
- **Stack:** L0 plus a dense embedder through `fastembed-rs` (trimmed features, `PreferCPU`), fused with RRF (k=60).
- **Starting model:** `bge-small-en-v1.5` int8 (`BGESmallENV15Q`): 384 dims, 512 tokens, MIT, about 34 MB on disk [S1][S47] [V].
- **Footprint** [Inf; verify in E7]:
  - RAM 100–250 MB including ORT;
  - about 2–5 s of CPU per 1k memories at ingest;
  - about 3–10 ms per query embedding;
  - 100k memories need about 154 MB of f32 vectors plus about 15 MB of HNSW.
- **Swap rule:** replace bge-small with granite-embedding-small-english-r2 (Apache-2.0, 8K context, better claimed CoIR) or EmbeddingGemma 2 text at 256 dims (Apache-2.0, strongest claimed code score) if either wins E1 by at least 2 nDCG points within the latency budget.
  - EmbeddingGemma 2 costs about 6× the compute: roughly 15–30 s per 1k memories and 174–314 MB of RAM.
  - It needs us to wire an ONNX graph through `ort` ourselves (or wait for fastembed support). Track fastembed-rs for it.

**Tier L1-lite — pure-Rust fallback** (no AVX2, musl or wasm, or a "no native code" policy)
- **Stack:** `model2vec-rs` with potion-retrieval-32M (prose) and/or potion-code-16M-v2 (code), fused with BM25 by RRF.
- **Footprint:** 30–130 MB, about 1–5 ms per 1k memories.
- **Expected quality:** a modest gain over BM25 alone (CoIR 43.36 vs 42.31 in the vendor's RRF test [S34] [C]). Do not expect transformer-level gains on prose [S67] [I].

**Reranker:** off by default. Allow it as an opt-in: ms-marco-MiniLM-L6-v2 int8 (23 MB) on the top 20, an estimated 20–60 ms [Inf]. Make it the default only if E5 justifies it.

**Avoid in laptop defaults:**
- Non-commercial licenses: all Jina models listed above, splade-v3, jina-v4 (Qwen Research License).
- EmbeddingGemma-300M, because the Gemma Terms of Use and the Hugging Face gate complicate auto-download in an open-source tool; EmbeddingGemma 2 replaces it under Apache-2.0.
- Qwen3-0.6B on CPU, at about 45–90 s per 1k memories.

### 11.2 Server mode (shared service for many agents)

- **Retrieval:** BM25 plus a stronger dense model plus a reranker on the top 20–50.
- **Dense model:**
  - **EmbeddingGemma 2 text** (Apache-2.0). Store 768 dims and index 256–512 dims as int8, or use Qdrant TurboQuant if Qdrant is the store.
  - Or **Qwen3-Embedding-0.6B** (Apache-2.0) at 1024 dims, truncated to 512 by MRL, if a GPU or Metal is available.
  - Run it through fastembed-rs's candle path (Qwen3 supports Metal and CUDA [S1]) or a TEI sidecar [S13].
- **Reranker:** Qwen3-Reranker-0.6B (Apache-2.0; candle in fastembed [S2]), or mxbai-rerank-base-v2 or gte-reranker-modernbert-base, selected by E5.
- **Hosted option:** the voyage-4 family:
  - embed documents with voyage-4 or voyage-4-large;
  - laptops can embed queries locally with **voyage-4-nano** (Apache-2.0) in the *same* space [S40][S41] [C];
  - price $0.02–0.12 per 1M tokens [S43] [V].
  - Verify the cross-model compatibility claim in E3 before relying on it.
- **Footprint** [Inf]: about 0.3–2.4 GB of model RAM. Vectors at 1M memories: 1.5 GB (768 dims, f16) or 0.5 GB (512 dims, int8) plus HNSW. A GPU or Metal is strongly preferred for Qwen3 and the 0.6B rerankers.

### 11.3 Model changes (both modes)

- Adopt the `space_id` scheme and the dual-write, backfill and RRF-across-spaces procedure in §8.3.
- Pin model revisions by SHA and keep the weight digest.
- Re-embedding budget: minutes for 100k memories with small models, hours for 1M memories with 0.3–0.6B models on CPU. Schedule it as idle-time background work on laptops and as a batch job (or a hosted Batch API) on servers.

---

## 12. Open questions

1. How does EmbeddingGemma 2's text-only ONNX export run under `ort`: inputs and outputs, whether pooling and the 512→768 projection are inside the graph, and q4 accuracy? When will fastembed-rs add it? [?]
2. What are the real laptop numbers for ORT CPU vs CoreML vs candle Metal, for bge-small, granite-small-r2 and EmbeddingGemma 2? How large is the binary-size delta when ORT is statically linked? [?]
3. Do CodeRankEmbed-style code embedders actually help on *natural-language memories that mention code*, or only on natural-language → code-snippet retrieval? (E1 per-query-type breakdown.)
4. Is RRF k=60 good for very short documents, or do we need a convex combination or adaptive weighting? Does the best tuning differ per scope (project vs organization corpus)? [?]
5. Does inference-free learned sparse retrieval (opensearch doc-v3-gte) beat a well-tuned BM25 on identifiers? [?]
6. How much does a "contextual" prefix help short memories? For example, prepending project, language and toolchain metadata before embedding and indexing, as in Anthropic's contextual retrieval [S62].
7. Does Voyage's cross-model space really hold for our data (nano queries against large documents)? What is the lock-in cost? [?]
8. Should we fine-tune a tiny memory-specific embedder and reranker (as Hindsight did) once we have logged query/click data? What licensing and governance does that require? [?]
9. Cohere's current embed pricing and model terms are not on its public pricing page [?].
10. Is it worth integrating ColBERT-style late interaction (NextPlaid / LateOn-Code-edge) for code-heavy scopes, given its 5–10× storage overhead? [?]

---

## 13. Sources (all checked 2026-11-10)

1. Anush008/fastembed-rs at commit `c6b02d8`: README (model lists L22–96; Qwen3 L258–324; BERT and Metal L351–380), `Cargo.toml` (L3, L37–86), `src/models/text_embedding.rs` (L9–102, L109–532), `src/text_embedding/init.rs` (L54–60) — https://github.com/Anush008/fastembed-rs
2. fastembed-rs pull requests (#323–#353, including #331 Qwen3-Reranker, #335 optional ONNX Runtime, #348 automatic device selection, #352 Metal BERT) — https://github.com/Anush008/fastembed-rs/pulls?q=is%3Apr
3. crates.io API metadata for fastembed, ort, ort-tract, ort-candle, candle-core, candle-transformers, tract-onnx, burn, burn-import, llama-cpp-2, embed_anything, model2vec-rs, tokenizers, tantivy, bm25, usearch, hnsw_rs, lancedb, qdrant-client, sqlite-vec, ollama-rs — https://crates.io/api/v1/crates/{name}
4. pykeio/ort at commit `6ed2423`: `Cargo.toml` (L27–31, L56, L71); `docs/content/misc/prebuilt-binaries.mdx` (L7–17); `docs/content/backends/index.mdx` (L19, L69–72) — https://github.com/pykeio/ort and https://ort.pyke.io/
5. ONNX Runtime v1.31.1 release assets — https://github.com/microsoft/onnxruntime/releases/tag/v1.31.1
6. MinishLab/model2vec-rs at commit `768bdf3`: README (L128, L201–245), `Cargo.toml` (L3, L33–41), LICENSE (MIT) — https://github.com/MinishLab/model2vec-rs
7. StarlightSearch/EmbedAnything at commit `d088134`: `Cargo.toml` (L14–19), `rust/Cargo.toml` (L20–99), README (L177–187) — https://github.com/StarlightSearch/EmbedAnything
8. candle-transformers model directory — https://github.com/huggingface/candle/tree/main/candle-transformers/src/models
9. sonos/tract README — https://github.com/sonos/tract
10. tracel-ai/burn README — https://github.com/tracel-ai/burn
11. utilityai/llama-cpp-rs examples (embeddings, reranker) — https://github.com/utilityai/llama-cpp-rs/tree/main/examples
12. Ollama API docs, "Generate Embeddings" (L1666–1712), `docs/capabilities/embeddings.mdx`, and release v0.40.16 — https://github.com/ollama/ollama/blob/main/docs/api.md
13. huggingface/text-embeddings-inference README and release v1.9.4 — https://github.com/huggingface/text-embeddings-inference
14. all-MiniLM-L6-v2 model card — https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2
15. bge-small-en-v1.5 model card (MTEB table) — https://huggingface.co/BAAI/bge-small-en-v1.5
16. nomic-embed-text-v1.5 model card — https://huggingface.co/nomic-ai/nomic-embed-text-v1.5
17. nomic-embed-text-v2-moe model card — https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe
18. EmbeddingGemma-300M model card — https://huggingface.co/google/embeddinggemma-300m (paper: https://arxiv.org/abs/2509.20354)
19. "Introducing EmbeddingGemma" (Google Developers Blog) — https://developers.googleblog.com/en/introducing-embeddinggemma/
20. EmbeddingGemma 2 model card — https://huggingface.co/google/embeddinggemma-2
21. EmbeddingGemma 2 launch blog — https://blog.google/innovation-and-ai/technology/developers-tools/embeddinggemma-2
22. Qwen3-Embedding-0.6B model card — https://huggingface.co/Qwen/Qwen3-Embedding-0.6B (paper: https://arxiv.org/abs/2506.05176)
23. snowflake-arctic-embed-m-v2.0 model card — https://huggingface.co/Snowflake/snowflake-arctic-embed-m-v2.0
24. snowflake-arctic-embed-xs model card (v1 family table) — https://huggingface.co/Snowflake/snowflake-arctic-embed-xs
25. jina-embeddings-v3 model card — https://huggingface.co/jinaai/jina-embeddings-v3
26. jina-embeddings-v4 model card (license section) — https://huggingface.co/jinaai/jina-embeddings-v4
27. jina-embeddings-v5-text-nano model card — https://huggingface.co/jinaai/jina-embeddings-v5-text-nano
28. jina-embeddings-v5-text-small model card — https://huggingface.co/jinaai/jina-embeddings-v5-text-small
29. jina-code-embeddings-0.5b model card — https://huggingface.co/jinaai/jina-code-embeddings-0.5b
30. mxbai-embed-large-v1 / mxbai-embed-xsmall-v1 model cards — https://huggingface.co/mixedbread-ai/mxbai-embed-large-v1 and https://huggingface.co/mixedbread-ai/mxbai-embed-xsmall-v1
31. gte-modernbert-base model card (MTEB, CoIR and gte-reranker table) — https://huggingface.co/Alibaba-NLP/gte-modernbert-base
32. granite-embedding-small-english-r2 model card — https://huggingface.co/ibm-granite/granite-embedding-small-english-r2
33. potion-base-8M and potion-retrieval-32M model cards — https://huggingface.co/minishlab/potion-base-8M and https://huggingface.co/minishlab/potion-retrieval-32M
34. potion-code-16M-v2 model card (CoIR table including BM25 and the hybrid) — https://huggingface.co/minishlab/potion-code-16M-v2
35. jina-embeddings-v2-base-code model card — https://huggingface.co/jinaai/jina-embeddings-v2-base-code
36. CodeRankEmbed model card — https://huggingface.co/nomic-ai/CodeRankEmbed
37. CoRNStack paper — https://arxiv.org/abs/2412.01007
38. nomic-embed-code model card — https://huggingface.co/nomic-ai/nomic-embed-code
39. voyage-code-3 blog — https://blog.voyageai.com/2024/12/04/voyage-code-3/
40. Voyage 4 blog (shared embedding space) — https://blog.voyageai.com/2026/01/15/voyage-4/
41. voyage-4-nano model card — https://huggingface.co/voyageai/voyage-4-nano
42. Voyage embeddings docs (model table) — https://docs.voyageai.com/docs/embeddings
43. Voyage pricing — https://docs.voyageai.com/docs/pricing
44. Voyage tokenization docs — https://docs.voyageai.com/docs/tokenization
45. CoIR benchmark paper — https://arxiv.org/abs/2407.02883
46. hs-embed-1 and hs-rerank-1 model cards, and the Hindsight repo — https://huggingface.co/hindsight-ai/hs-embed-1, https://huggingface.co/hindsight-ai/hs-rerank-1, https://github.com/vectorize-io/hindsight
47. Hugging Face model API (parameter counts, license tags, gating, file sizes via `?blobs=true`) — https://huggingface.co/api/models/{id}
48. bge-reranker-v2-m3 model card — https://huggingface.co/BAAI/bge-reranker-v2-m3
49. jina-reranker-v3 model card (comparison table including CoIR) — https://huggingface.co/jinaai/jina-reranker-v3
50. jina-reranker-v3.5 model card — https://huggingface.co/jinaai/jina-reranker-v3.5
51. mxbai-rerank-base-v2 model card — https://huggingface.co/mixedbread-ai/mxbai-rerank-base-v2
52. cross-encoder/ms-marco-MiniLM-L6-v2 model card — https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2
53. Qwen3-Reranker-0.6B model card — https://huggingface.co/Qwen/Qwen3-Reranker-0.6B
54. gte-reranker-modernbert-base model card — https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base
55. answerai-colbert-small-v1 model card and artifact metadata (dim 96) — https://huggingface.co/answerdotai/answerai-colbert-small-v1
56. lightonai/pylate (MIT) — https://github.com/lightonai/pylate
57. lightonai/next-plaid (NextPlaid and ColGREP; Apache-2.0) README; LateOn-Code-edge — https://github.com/lightonai/next-plaid and https://huggingface.co/lightonai/LateOn-Code-edge
58. LateOn-Code model card — https://huggingface.co/lightonai/LateOn-Code
59. MinishLab/semble README — https://github.com/MinishLab/semble
60. BEIR paper — https://arxiv.org/abs/2104.08663
61. Bruch et al., "An Analysis of Fusion Functions for Hybrid Retrieval" — https://arxiv.org/abs/2210.11934
62. Anthropic, "Introducing Contextual Retrieval" — https://www.anthropic.com/news/contextual-retrieval
63. SPLADE v2 paper — https://arxiv.org/abs/2109.10086
64. prithivida/Splade_PP_en_v1 model card — https://huggingface.co/prithivida/Splade_PP_en_v1
65. opensearch-neural-sparse-encoding-doc-v3-gte model card — https://huggingface.co/opensearch-project/opensearch-neural-sparse-encoding-doc-v3-gte
66. Hugging Face, "Binary and Scalar Embedding Quantization" — https://huggingface.co/blog/embedding-quantization (source: https://github.com/huggingface/blog/blob/main/embedding-quantization.md)
67. Hugging Face, "Train 400x faster Static Embedding Models" (NanoBEIR and CPU/GPU throughput table) — https://huggingface.co/blog/static-embeddings
68. Qdrant, "Migrate to a New Embedding Model with Zero Downtime" — https://qdrant.tech/documentation/tutorials-operations/embedding-model-migration/
69. Qdrant docs, Collections → "Update Vector Schema" (v1.18) — https://qdrant.tech/documentation/manage-data/collections/#update-vector-schema
70. Qdrant 1.16 release blog (conditional updates) — https://qdrant.tech/blog/qdrant-1.16.x/
71. Qdrant 1.18 release blog (TurboQuant, named-vector schema changes) — https://qdrant.tech/blog/qdrant-1.18.x/
72. LanceDB, "Managing Embeddings" (`docs/web/embedding/index.mdx`) and `add_columns` — https://github.com/lancedb/lancedb/blob/main/docs/web/embedding/index.mdx
73. Drift-Adapter paper — https://arxiv.org/abs/2509.23471
74. "Learning Backward Compatible Embeddings" — https://arxiv.org/abs/2206.03040
75. OpenAI API pricing (embeddings rows) — https://developers.openai.com/api/docs/pricing
76. OpenAI embeddings guide (tokens, max input, dimensions) — https://developers.openai.com/api/docs/guides/embeddings
77. Gemini Developer API pricing (Gemini Embedding and Gemini Embedding 2) — https://aistudio.google.com/docs/pricing (formerly https://ai.google.dev/gemini-api/docs/pricing)
78. gemini-embedding-001 model page — https://aistudio.google.com/docs/models/gemini-embedding-001
79. Cohere pricing page — https://cohere.com/pricing
80. MMTEB paper — https://arxiv.org/abs/2502.13595
81. Hugging Face, "Introducing RTEB" (October 1, 2025) — https://huggingface.co/blog/rteb
82. hnswlib `ALGO_PARAMS.md` (memory of about M × 8–10 bytes per element) — https://github.com/nmslib/hnswlib/blob/master/ALGO_PARAMS.md
83. quickwit-oss/tantivy README — https://github.com/quickwit-oss/tantivy
84. SQLite FTS5 documentation (`bm25()` function) — https://www.sqlite.org/fts5.html
85. Michael-JB/bm25 crate README — https://github.com/Michael-JB/bm25
86. Matryoshka Representation Learning paper — https://arxiv.org/abs/2205.13147
