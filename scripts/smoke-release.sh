#!/usr/bin/env bash
# Exercise the extracted release binaries on their native runner.
set -euo pipefail
BIN_DIR="$(cd "${1:?extracted binary directory}" && pwd)"
RUN_DIR="$(mktemp -d "/tmp/hm-smoke-XXXXXX")"
DAEMON_PID=""
cleanup() {
  if [[ -n "$DAEMON_PID" ]]; then kill "$DAEMON_PID" 2>/dev/null || true; wait "$DAEMON_PID" 2>/dev/null || true; fi
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT
HM="$BIN_DIR/hivemind"
"$HM" --version
"$BIN_DIR/hivemindd" --version
"$HM" --dir "$RUN_DIR/store" --json remember "Release smoke durable memory" --scope project:release > "$RUN_DIR/receipt.json"
"$HM" --dir "$RUN_DIR/store" --json recall "Release smoke" > "$RUN_DIR/recall.json"
python3 - "$RUN_DIR" <<'PY'
import json, pathlib, sys
p=pathlib.Path(sys.argv[1]); receipt=json.loads((p/'receipt.json').read_text()); recall=json.loads((p/'recall.json').read_text())
assert any(c['id']==receipt['claim'] for c in recall['cards']), recall
PY
"$HM" --dir "$RUN_DIR/store" verify
"$HM" --dir "$RUN_DIR/store" backup "$RUN_DIR/backup"
"$HM" --dir "$RUN_DIR/backup" verify
"$HM" --dir "$RUN_DIR/backup" --json recall "Release smoke" > "$RUN_DIR/restored.json"
python3 - "$RUN_DIR/restored.json" <<'PY'
import json, sys
assert json.load(open(sys.argv[1]))['cards']
PY
"$BIN_DIR/hivemindd" --dir "$RUN_DIR/store" --idle-timeout 0 > "$RUN_DIR/daemon.log" 2>&1 &
DAEMON_PID=$!
for _ in $(seq 1 100); do
  [[ -S "$RUN_DIR/store/run/hivemindd.sock" ]] && break
  kill -0 "$DAEMON_PID"
  sleep 0.05
done
"$HM" --dir "$RUN_DIR/store" --json remember "Second agent shares this daemon" --scope project:release > "$RUN_DIR/daemon-receipt.json"
"$HM" --dir "$RUN_DIR/store" --json recall "Second agent" > "$RUN_DIR/daemon-recall.json"
printf '{"prompt":"Second agent","cwd":"%s"}\n' "$RUN_DIR" | "$HM" --dir "$RUN_DIR/store" brief --scope project:release --hook-json > "$RUN_DIR/brief-hook.json"
python3 - "$RUN_DIR" <<'PY'
import json, pathlib, sys
p=pathlib.Path(sys.argv[1]); receipt=json.loads((p/'daemon-receipt.json').read_text()); recall=json.loads((p/'daemon-recall.json').read_text())
brief=json.loads((p/'brief-hook.json').read_text())
assert any(c['id']==receipt['claim'] for c in recall['cards']), recall
assert 'Second agent shares this daemon' in brief.get('additionalContext', ''), brief
PY
printf 'Release smoke passed: remember, recall, verify, backup/restore, shared daemon.\n'
