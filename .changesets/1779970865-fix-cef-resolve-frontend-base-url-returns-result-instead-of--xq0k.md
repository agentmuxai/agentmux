---
type: patch
---

fix(cef): resolve_frontend_base_url returns Result instead of silently emitting localhost:5173 in production

The function's no-frontend-bundle fallback historically returned
`http://localhost:<vite_port>` — the dev Vite URL — when
`frontend/index.html` was missing next to the exe. In production
nothing is listening on that port; the new browser would abort with
ERR_ABORTED, terminate its renderer, and trigger the unbounded crash
loop that caused the 2026-05-28 incident (139k crashes in 22 min).
Same fallback was already documented as buggy in
`docs/analyses/ANALYSIS_DEV_VITE_PORT_HARDCODE_2026-05-26.md` but that
fix only patched the dev branch.

Change signature to `Result<String, FrontendUrlError>` with variants
`AssetsMissing { checked_path }` and `ExeUnresolvable`. Add
`assets_missing_data_url(&err)` helper that returns a self-contained
`data:` URL rendering a static "AgentMux install is broken" page —
no auto-reload, no link back to the broken install, so navigating to
it can never trigger another crash loop.

All five callers updated to handle the error: open_window,
window_pool warmup, tab tear-off (drag.rs), floating pane, and the
renderer-crash recovery page. The crash-recovery path is
special-cased — if the recovery handler itself can't resolve a URL,
it loads the assets-missing page directly so the Reload button
never points at a network URL that would crash again.

Promotes `html_escape` and `js_string_literal` in `client/helpers.rs`
from `pub(super)` to `pub(crate)`, and the `helpers` module itself
to `pub(crate) mod`, so window.rs can share them. No behavior change.

Related: #1117 (follow-up tracking issue),
`docs/retro/retro-portable-rm-running-install-2026-05-28.md`.
