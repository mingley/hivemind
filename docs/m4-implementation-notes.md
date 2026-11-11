# M4 implementation notes

Status: **implemented** (2026-11-11). Milestone **M4 Retrieval depth** from
[06](06-prototype-plan.md), mechanisms from [08](08-hippocampal-design.md).

## Delivered

| Piece | Where |
|---|---|
| `EmbeddingSpace` / `Embedder` / `HashEmbedder` | `hivemind-engine::vector` |
| Exact f32 + int8 scan with rescoring | `VectorIndex::search` |
| RRF fusion in recall | `recall` + `rrf_fuse` (`rank-v1`) |
| Key expansion (C1) | `keys::expand` on claim index text |
| Entity boost / abstention / temporal neighbors | `Query` fields |
| Contrasts + supersede context match (C5/C6) | `EdgeKind::ContrastsWith`, `remember` |
| ACT-R-style activation (C14) | `row_multiplier` |

## Follow-ups

- Plug a real pure-Rust model (`model2vec-rs` / candle) behind `Embedder`.
- Persist vectors across reopen (today: reindex via `set_embedder`).
- ANN for large scopes (M6).
