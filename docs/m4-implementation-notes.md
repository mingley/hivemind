# M4 implementation notes

Status: **implemented** (2026-11-11). Milestone **M4 Retrieval depth** from
[06](06-build-plan.md), mechanisms from [08](08-hippocampal-design.md).

## Delivered

| Piece | Where |
|---|---|
| `EmbeddingSpace` / `Embedder` / `HashEmbedder` | `hivemind-engine::vector` |
| `EmbedderChoice` / `build_embedder` / optional `Model2VecEmbedder` | `vector` + cargo feature `embeddings` |
| CLI/env wiring (`HIVEMIND_EMBEDDER`, …) | `hivemind::embedder`, daemon / serve / CLI |
| Exact f32 + int8 scan with rescoring | `VectorIndex::search` |
| RRF fusion in recall | `recall` + `rrf_fuse` (`rank-v1`) |
| Key expansion (C1) | `keys::expand` on claim index text |
| Entity boost / abstention / temporal neighbors | `Query` fields |
| Contrasts + supersede context match (C5/C6) | `EdgeKind::ContrastsWith`, `remember` |
| ACT-R-style activation (C14) | `row_multiplier` |

**Lean default:** no embedder attached; remember/recall are lexical-only. See
[operations.md](operations.md#embeddings-optional-lean-default-is-lexical-only).

## Follow-ups

- Persist vectors across reopen (today: reindex via `set_embedder`).
- Optional ONNX / fastembed path if F4 hybrid evidence warrants a heavier default.
- ANN for large scopes (M6) — LSH exists; tune thresholds.
