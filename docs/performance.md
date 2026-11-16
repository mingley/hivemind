# Performance — v0.1.0

Measured on 2026-11-16, Linux/x86_64 cloud workspace, release build, no
embeddings. Claims are synthetic code/debugging memories across 20 project
scopes. Measurements are engine calls; they do not include agent/network time.
The generator seed is `0x9E3779B97F4A7C15`. Filesystem caches are warm.

## Same-host comparison at 20,000 claims

The baseline is commit `a4220bb`. Both runs use the same claim generator,
queries and release settings. Latencies vary with shared-host load; the table
uses the final run, rather than the fastest observed sample.

| Metric | Before | v0.1.0 |
|---|---|---|
| Scoped recall p95 (200 queries) | 0.315 ms | 0.168 ms |
| Global recall p95 (100 queries) | 2.926 ms | 1.004 ms |
| Durable write p95 (200 writes) | 1.205 ms | 0.789 ms |
| Replay RSS | 61.5 MiB | 38.4 MiB |
| Ingest without fsync | 4,714 claims/s | 6,276 claims/s |

## Default-capacity latency: 100,000 claims

`cargo run --release --locked -p hivemind-engine --example bench -- 100000 --check`
exercises the normal `remember` path, reopens the store, recalls sampled
queries, expands full claims by id, and acknowledges 200 additional writes
only after fsync. Benchmark options reserve those 200 extra rows; the product's
default admission cap remains 100,000.

| Metric | p50 | p95 | p99 | CI p95 ceiling |
|---|---|---|---|---|
| Scoped recall | 0.691 ms | 1.098 ms | 1.435 ms | 50 ms |
| Global recall | 6.984 ms | 8.286 ms | 9.676 ms | 50 ms |
| Expand full claim | 0.036 ms | 0.154 ms | 0.278 ms | 20 ms |
| Durable write | 1.620 ms | 3.074 ms | 3.391 ms | 20 ms |

All 200 sampled scoped queries returned a card. Replay took 2.17 s; process
RSS after replay was 122.8 MiB and 124.8 MiB after query/write samples. Normal
write-path ingest without fsync averaged 1,403 claims/s across the growing
100k store. This is a synthetic retrieval/latency check, not an agent outcome
study or a concurrent-service throughput benchmark.

CI runs this 100k workload with explicit latency ceilings and a 256 MiB
synthetic-process RSS budget, plus workspace and durability tests.

## Replay-only footprint and scale

```sh
cargo run --release --locked -p hivemind-engine --example rss_harness -- \
  --seed-log --max-rss-mb 2048 10000 100000 1000000
```

`--seed-log` validates/encodes claim events directly into the log without
running similarity search during generation. The store still performs full
validated replay to build its projections. These numbers measure replay and
footprint, not normal ingest speed. RSS is `ps -o rss=` KiB divided by 1024.

| Claims | Before RSS | v0.1.0 RSS | v0.1.0 replay |
|---|---|---|---|
| 10,000 | 25.8 MiB | 15.8 MiB | 0.19 s |
| 100,000 | 209.1 MiB | 117.0 MiB | 2.26 s |
| 1,000,000 | 1,675.8 MiB | 882.2 MiB | 23.99 s |

## Changes and operating envelope

- Filter irrelevant scopes/statuses before BM25 scoring while preserving the
  global corpus statistics. Tests compare filtered scores with unrestricted
  scores and ensure each candidate is checked only once.
- Inline singleton postings and compact dictionary keys; sort term frequencies
  without allocating a per-document hash table. Token admission at the cap is
  deterministic across replay.
- Share common applicability maps (bounded cache) and allocate lifecycle
  details only for claims that need them.
- Rebuild sparse seek anchors during log validation and maintain them on append.
  Reads seek near the requested offset, then validate the selected records.
  Full replay and `verify` still scan/validate the complete log. Segment and
  event formats are unchanged, including erasure replacements.
- Reuse the client socket reader and reap spawned daemon exits.

Provision at least 512 MiB per active store plus request/backup/embedding
headroom. Unique contexts, large claims, history and vocabulary alter RSS;
these synthetic budgets are not absolute memory bounds. Stores with an
explicitly raised 1M admission limit should provision at least 2.5 GiB per
store and allow ~24 s replay on a host like this one. Default remains 100k.
The 50 MB laptop goal remains unmet at 100k. Lexical search remains the default;
embeddings are optional and require a separate build/model budget.
