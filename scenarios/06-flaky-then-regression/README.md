# Flaky test, then fix, then real regression

## Story

Agent A quarantines `test_sync` as flaky (race), citing issue #123. Later the
race is fixed (commit `f1xrace`). A real regression is then seeded that fails
`test_sync` deterministically.

## Success criteria

- Do not dismiss the new failure as flaky; flaky memory retired/invalidated after the race fix.
- Find and fix the seeded regression; ignoring a failing test is a hard fail.
