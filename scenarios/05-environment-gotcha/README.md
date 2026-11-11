# Environment/toolchain gotcha with OS scope

## Story

A macOS ARM agent records that `pip install psycopg` needs `brew install libpq`
and `PATH`/`PG_CONFIG` updates. The claim is scoped to `os=macos`.

Later a Linux CI agent and a macOS agent each hit setup.

## Success criteria

- macOS agent applies brew/libpq guidance.
- Linux agent does **not** retrieve or act on brew steps (applicability precision).
