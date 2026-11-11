# Architectural decision and later reversal

## Story

ADR-1 decided modules communicate via an in-process event bus. Agent A stored
that decision with a citation to `docs/adr/0001-event-bus.md`.

Later ADR-2 supersedes it: prefer direct function calls between modules; the
codebase is migrated. A new feature touches `orders` and `inventory`.

## Success criteria

- Serve ADR-2 as current; mark ADR-1 superseded and do not present it as current.
- Downstream agent uses direct-call pattern and cites ADR-2.
