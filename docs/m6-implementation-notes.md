# M6 implementation notes (engine growth)

Contract snapshot for the M6 deliverables in
[06](06-build-plan.md#m6-status-engine-growth-done-2026-11-11).

## Snapshots

- `Store::checkpoint()` → `snapshots/<next_offset:020>.hsnap`
- Magic `HMSNAP\0\x01`, BLAKE3 over payload; contiguous events `[0, next_offset)`
- Open: apply pack, then `Log::open_from(..., apply_from)` skips re-apply
- Log is still fully checksum-validated

## Mmap lexical segments

- `LexicalSegment::write_sealed` / `open` (`memmap2`)
- Magic `HMLXSEG\x01`; BM25 matches in-memory `LexicalIndex`
- `Store::seal_lexical()` → `indexes/lexical-<offset>.seg`

## ANN

- `VectorAnn`: random-hyperplane LSH, exact cosine rescoring
- `Store::build_ann()`; recall uses `search_ann` when tables are built
- Falls back to exact scan below the row threshold or if ANN is absent

## Deterministic simulation

- `tests/m6_sim.rs`: seeded LCG ops, drop store (crash), reopen, verify
  acknowledged offsets and backup+reopen claim counts
