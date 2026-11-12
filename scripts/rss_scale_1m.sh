#!/usr/bin/env bash
# Mid/large-scale RSS + replay budget runner for hivemind-engine.
#
# Default: measure 10k and 100k (prefer mid-scale land). Optionally try 1M
# when HIVEMIND_RSS_TRY_1M=1 and the host looks like it can finish quickly.
#
# Usage:
#   ./scripts/rss_scale_1m.sh
#   HIVEMIND_RSS_DIR=/tmp/hm-rss ./scripts/rss_scale_1m.sh
#   HIVEMIND_RSS_TRY_1M=1 ./scripts/rss_scale_1m.sh
#   HIVEMIND_RSS_NS="10000 100000 250000" ./scripts/rss_scale_1m.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DIR="${HIVEMIND_RSS_DIR:-$(mktemp -d /tmp/hivemind-rss-XXXXXX)}"
NS_RAW="${HIVEMIND_RSS_NS:-10000 100000}"
TRY_1M="${HIVEMIND_RSS_TRY_1M:-0}"
PROGRESS="${HIVEMIND_RSS_PROGRESS:-1}"

mkdir -p "$DIR"
LOG="$DIR/rss_scale.log"

mem_avail_mb() {
  if [[ -r /proc/meminfo ]]; then
    awk '/MemAvailable:/ {printf "%d", $2/1024; exit}' /proc/meminfo
  else
    echo 0
  fi
}

# Heuristic: only append 1M when explicitly requested *and* we have headroom.
# Prior 100k post-mitigation RSS is ~200 MB; 1M synthetic may need multi-GB.
if [[ "$TRY_1M" == "1" ]]; then
  avail="$(mem_avail_mb)"
  if (( avail >= 4096 )); then
    if [[ " $NS_RAW " != *" 1000000 "* ]]; then
      NS_RAW="$NS_RAW 1000000"
    fi
    echo "rss_scale: TRY_1M=1 and MemAvailable≈${avail}MB — including 1M checkpoint"
  else
    echo "rss_scale: TRY_1M=1 but MemAvailable≈${avail}MB < 4096 — skipping 1M (mid-scale only)"
  fi
fi

PROGRESS_FLAG=()
if [[ "$PROGRESS" == "1" ]]; then
  PROGRESS_FLAG=(--progress)
fi

echo "rss_scale: host=$(uname -s)/$(uname -m) dir=$DIR ns=$NS_RAW"
echo "rss_scale: building release example rss_harness…"
cargo build --release -p hivemind-engine --example rss_harness

BIN="$ROOT/target/release/examples/rss_harness"
# Fall back to cargo run if the example binary path differs across Cargo versions.
if [[ ! -x "$BIN" ]]; then
  echo "rss_scale: binary not at $BIN — using cargo run"
  # shellcheck disable=SC2086
  cargo run --release -p hivemind-engine --example rss_harness -- \
    --dir "$DIR" ${PROGRESS_FLAG[@]+"${PROGRESS_FLAG[@]}"} $NS_RAW | tee "$LOG"
else
  # shellcheck disable=SC2086
  "$BIN" --dir "$DIR" ${PROGRESS_FLAG[@]+"${PROGRESS_FLAG[@]}"} $NS_RAW | tee "$LOG"
fi

echo
echo "rss_scale: log → $LOG"
echo "rss_scale: target ≤ 50 MB idle RSS (docs/01); see PASS/MISS lines above"
