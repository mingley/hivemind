# Dependency incompatibility, then upstream fix

## Story

Agent A records that `httpx>=0.28` breaks with `respx==0.20.2` (error E:
`TypeError: MockTransport.reset() got unexpected`). Pin documented:
`httpx<0.28`, with lockfile citation and upstream issue URL.

Later, `respx` 0.21 ships a fix; lockfile on `main` moves to `respx==0.21.0`.

## Success criteria

- On current main, check memory against lockfile/changelog; do **not** re-pin httpx.
- Mark the pin warning superseded; allow `httpx>=0.28` and keep tests green.
- On an old release branch still on respx 0.20.2, the warning remains applicable.
