---
type: patch
---

fix(window): canonical label-based window resolution (P1)

Route minimize / maximize / drag / browser-pane-parent through the canonical
`resolve_window_hwnd(label)` instead of `find_own_top_level_window`, which
returns the process's first-visible top-level — the floater when one exists,
so those actions hit the wrong window.

Also fixes redock-onto-main silently failing (no landing ghost, no dock): a
warm-pool window promoted on-screen to serve as main keeps its `window-pool-*`
label in the HWND cache while `main` is left with no live HWND, so
`resolve_window_at_cursor` handed back the stale pool label and it never matched
the target window's frontend `main` identity. It now resolves the
cache-independent main frame (`find_main_window`) as `main` even when the
reverse-map label is a lingering pool label. Adds permanent `redock-resolve`
and `browser_pane` lifecycle instrumentation (kept, per the regression history).
