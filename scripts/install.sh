#!/bin/sh
# Install official release binaries after verifying the published archive hash.
set -eu
VERSION="${HIVEMIND_VERSION:-latest}"
INSTALL_DIR="${HIVEMIND_INSTALL_DIR:-$HOME/.local/bin}"
case "$(uname -s)" in
  Darwin) OS=macos ;;
  Linux) OS=linux ;;
  *) printf '%s\n' 'Supported: macOS and Linux (Windows users can use WSL2).' >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) ARCH=aarch64 ;;
  x86_64|amd64) ARCH=x86_64 ;;
  *) printf '%s\n' 'Unsupported CPU architecture.' >&2; exit 1 ;;
esac
case "$VERSION" in *[!A-Za-z0-9._-]*|'') printf '%s\n' 'Invalid release version.' >&2; exit 1 ;; esac
ARCHIVE="hivemind-$OS-$ARCH.tar.gz"
BASE=https://github.com/mingley/hivemind/releases
if [ "$VERSION" = latest ]; then BASE="$BASE/latest/download"; else BASE="$BASE/download/$VERSION"; fi
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/hivemind-install-XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM
command -v curl >/dev/null
if command -v sha256sum >/dev/null 2>&1; then HASH=sha256sum; else command -v shasum >/dev/null; HASH=shasum; fi
curl --proto '=https' --tlsv1.2 -fsSL "$BASE/$ARCHIVE" -o "$TMP_DIR/$ARCHIVE"
curl --proto '=https' --tlsv1.2 -fsSL "$BASE/SHA256SUMS" -o "$TMP_DIR/published-checksums"
awk -v name="$ARCHIVE" '$2 == name { print; found=1 } END { if (!found) exit 1 }' "$TMP_DIR/published-checksums" > "$TMP_DIR/archive.sha256"
(cd "$TMP_DIR"; if [ "$HASH" = sha256sum ]; then sha256sum -c archive.sha256; else shasum -a 256 -c archive.sha256; fi)
tar -xzf "$TMP_DIR/$ARCHIVE" -C "$TMP_DIR" hivemind hivemindd LICENSE SHA256SUMS
(cd "$TMP_DIR"; if [ "$HASH" = sha256sum ]; then sha256sum -c SHA256SUMS; else shasum -a 256 -c SHA256SUMS; fi)
# Refuse installation if this archive cannot execute on this host.
"$TMP_DIR/hivemind" --version
"$TMP_DIR/hivemindd" --version
mkdir -p "$INSTALL_DIR"
for BIN in hivemind hivemindd; do
  cp "$TMP_DIR/$BIN" "$INSTALL_DIR/.$BIN.installing"
  chmod 755 "$INSTALL_DIR/.$BIN.installing"
  mv -f "$INSTALL_DIR/.$BIN.installing" "$INSTALL_DIR/$BIN"
done
cp "$TMP_DIR/LICENSE" "$INSTALL_DIR/hivemind-LICENSE"
printf 'Installed to %s\nAdd this directory to PATH, then run: hivemind --help\n' "$INSTALL_DIR"
