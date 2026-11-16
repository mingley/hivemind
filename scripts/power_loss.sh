#!/usr/bin/env bash
# Build LazyFS beforehand; run from the repository root. No privilege escalation
# or host configuration is performed here. CI installs FUSE; the mount and tests
# run as the same user.
set -euo pipefail
: "${LAZYFS_SOURCE:?path to a built dsrhaslab/lazyfs checkout}"
ROOT="$(pwd)"
RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/hivemind-power-loss-XXXXXX")"
mkdir -p "$RUN_DIR/mount" "$RUN_DIR/backing"
cat > "$RUN_DIR/config.toml" <<TOML
[faults]
fifo_path="$RUN_DIR/commands.fifo"
fifo_path_completed="$RUN_DIR/completed.fifo"
[cache]
apply_eviction=false
[cache.simple]
custom_size="64mb"
blocks_per_page=1
[filesystem]
log_all_operations=false
logfile="$RUN_DIR/lazyfs.log"
TOML
cleanup() {
  fusermount3 -u "$RUN_DIR/mount" 2>/dev/null || true
  if [[ -n "${LAZYFS_PID:-}" ]]; then kill "$LAZYFS_PID" 2>/dev/null || true; wait "$LAZYFS_PID" 2>/dev/null || true; fi
  if [[ -f "$RUN_DIR/lazyfs.log" ]]; then cp "$RUN_DIR/lazyfs.log" "$ROOT/lazyfs-test.log"; fi
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT
(cd "$LAZYFS_SOURCE/lazyfs" && exec ./build/lazyfs "$RUN_DIR/mount" --config-path "$RUN_DIR/config.toml" -o modules=subdir -o "subdir=$RUN_DIR/backing" -s -f) &
LAZYFS_PID=$!
for _ in $(seq 1 100); do
  if mountpoint -q "$RUN_DIR/mount" && [[ -p "$RUN_DIR/commands.fifo" && -p "$RUN_DIR/completed.fifo" ]]; then break; fi
  kill -0 "$LAZYFS_PID"
  sleep 0.1
done
mountpoint -q "$RUN_DIR/mount"
export HIVEMIND_POWER_LOSS_ROOT="$RUN_DIR/mount"
export HIVEMIND_LAZYFS_COMMAND="$RUN_DIR/commands.fifo"
export HIVEMIND_LAZYFS_COMPLETED="$RUN_DIR/completed.fifo"
timeout 180 cargo test --locked -p hivemind-engine --test power_loss -- --ignored --nocapture
