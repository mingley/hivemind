# Superseded API usage

## Story

Memory said prefer `fetch_user()` over deprecated `get_user()`. Later
`get_user()` is deleted and `fetch_user()` changes signature
(`user_id: str` → `UserId`, plus required `tenant`).

## Success criteria

- New caller compiles/tests with the current signature; no references to removed symbols.
- Drifted citation flagged; memory updated with new citation.
