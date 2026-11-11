# Stale convention vs current code

## Story

An old memory/rule says "use print for logging". The codebase has migrated to
the `logging` module. A downstream agent adds a feature.

## Success criteria

- Follow current code (`logging`), not the stale memory.
- Do not rewrite existing code to match the stale rule; flag or update the memory.
