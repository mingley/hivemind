# Download and try Hivemind v0.1.0

Native binaries are available for macOS 13+ (Apple Silicon and Intel) and Linux
with glibc 2.35+ (x86-64 and ARM64). Windows users can use the Linux build in WSL2.
No Rust compiler, model download, or external database is required.

## Install

Download the installer from the release, then run it:

```sh
curl -fsSL https://github.com/mingley/hivemind/releases/download/v0.1.0/install.sh -o /tmp/hivemind-install.sh
HIVEMIND_VERSION=v0.1.0 sh /tmp/hivemind-install.sh
export PATH="$HOME/.local/bin:$PATH"
hivemind --version
```

The installer selects your OS/CPU, verifies SHA-256 checksums, and installs
`hivemind` and `hivemindd` into `~/.local/bin`. To choose another destination,
set `HIVEMIND_INSTALL_DIR=/your/bin`. Both binaries must stay in the same
directory so MCP can find the daemon. Add that directory to your shell's PATH
for future sessions. The macOS binaries are unsigned; an independently imposed
Gatekeeper policy can require approving the downloaded binaries before use.

Manual download: https://github.com/mingley/hivemind/releases/tag/v0.1.0
Each archive contains both binaries, the MIT license, and binary checksums.
The release's `SHA256SUMS` also verifies the archives themselves.

## A one-minute test

Use an isolated temporary store:

```sh
TEST_STORE="$(mktemp -d)"
hivemind --dir "$TEST_STORE" remember "Our test command is cargo test --workspace" --scope project:demo
hivemind --dir "$TEST_STORE" recall "test command"
hivemind --dir "$TEST_STORE" verify
```

Expect a memory card containing your statement and a successful integrity check.
The normal persistent store defaults to `~/.hivemind/store`; omit `--dir` to use it.

## Connect an agent

For Codex CLI:

```sh
codex mcp add hivemind -- "$HOME/.local/bin/hivemind" mcp
codex mcp list
```

For Claude Code:

```sh
claude mcp add --transport stdio --scope user hivemind -- "$HOME/.local/bin/hivemind" mcp
claude mcp list
```

Restart the agent, then ask it to remember a project fact, recall it, and expand
it with `show`. Agents using the same store share one daemon automatically.
If an agent's sandbox restricts Unix sockets, allow its configured store socket
or run the MCP integration outside that restriction; details are in
https://github.com/mingley/hivemind/blob/v0.1.0/docs/setup-mcp.md.

## Supported deployment

This release supports local use and bounded single-node team services. Default:
100,000 claims per store, lexical search, optional embeddings off. Provision at
least 512 MiB per active tenant store plus request/backup overhead. History and
erasure tombstones count toward the cap. The old 50 MB goal is not a requirement
for this supported server envelope and remains unmet at 100k.

For remote teams, configure server-owned tenant grants and a TLS reverse proxy;
use machine OAuth credentials for agents and retain the bootstrap bearer for
administration. Complete backups include log, manifest, and evidence blobs.
There is no automatic multi-node failover. Setup and restore instructions:
https://github.com/mingley/hivemind/blob/v0.1.0/docs/operations.md.
