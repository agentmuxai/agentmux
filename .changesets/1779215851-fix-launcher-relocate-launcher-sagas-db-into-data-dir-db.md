---
type: patch
---

fix(launcher): relocate launcher-sagas.db into `<data-dir>/db/`

Closes audit item §8.3. The launcher saga log previously lived
directly in `<data-dir>/launcher-sagas.db` while srv put all its
SQLite files under `<data-dir>/db/`. The new
`data_dir::launcher_saga_log_path()` returns the canonical
`<data-dir>/db/launcher-sagas.db` and performs a one-shot back-
compat rename from the legacy location on first launch.

Idempotent + safe to call repeatedly; +4 unit tests cover fresh
install, legacy migration, both-files-present, repeated calls.
Audit doc also corrected: §8.2 (duplicate saga tables) was a
false alarm — retracted.
