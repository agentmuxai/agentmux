# Spec: Modularize `agentmux-cef/src/client/mod.rs`

**Date:** 2026-07-02
**File:** `agentmux-cef/src/client/mod.rs` (2,377 lines)
**Type:** Pure reorganization — zero logic changes, zero public API changes
**Tier:** Large

---

## Current state

- **2,377 lines:** ~2,273 impl + 104 inline tests (5 tests)
- **Already partially modularized** — `client/` also contains `handlers.rs` (507, CEF trait wrappers), `helpers.rs` (106), `wndproc.rs` (489, Windows-only). This spec finishes the job by breaking up `mod.rs` itself.
- The handler logic lives in **inherent `impl AgentMuxHandler { ... }` blocks** grouped by CEF concern. **Multiple inherent impl blocks across files ARE allowed in Rust** — so unlike shell.rs, the methods can be distributed cleanly.
- Platform code: Windows blocks scattered in `on_after_created` (icon/taskbar/focus hooks, SetWindowTextW, splash event, HWND cache) + macOS/Linux splash signals. `wndproc.rs` already isolates the bulk of Windows subclassing. Compile-verify Windows locally; CI covers ubuntu/mac.

## Public API surface (must remain re-exported from `mod.rs`)

Consumed by: `app.rs` (`use crate::client::*`), `creation.rs`, `window_pool.rs`, `floating_pane.rs`, `commands/window/meta.rs`.

- `AgentMuxClient` (re-exported from `handlers.rs` — unchanged)
- `AgentMuxHandler` (pub struct, core state)
- `dlog` (pub fn)
- `helpers::{js_string_literal, html_escape, backend_close_window}` (pub(crate), unchanged)
- `wndproc::install_main_window_floater_cascade_hook` (Windows, unchanged)
- `ClosePoolBrowserTask`

## Proposed layout

Keep `handlers.rs`, `helpers.rs`, `wndproc.rs` **unchanged**. Split `mod.rs` into:

```
agentmux-cef/src/client/
├── mod.rs            (~450: AgentMuxHandler struct def, constructors, window_label_for,
│                      ClosePoolBrowserTask, constants, dlog, mod decls + pub use re-exports)
├── lifecycle.rs      (~900: impl AgentMuxHandler { on_after_created, do_close,
│                      on_before_close, on_before_popup } — window/pool/cascade/quit-gate)
├── display.rs        (~126: impl AgentMuxHandler { on_title_change, on_favicon_urlchange })
├── navigation.rs     (~299: impl AgentMuxHandler { on_loading_state_change, on_load_end,
│                      on_load_error } — IPC injection, error pages, splash signals)
├── crash_recovery.rs (~520: impl AgentMuxHandler { on_render_process_terminated,
│                      on_auth_credentials } — crash/memory budgets, auth registry)
├── recovery_pages.rs (~190: free fns crash_loop_terminal_page, memory_paused_page,
│                      url_on_origin, record_memory_pause, recovery_navigation_url)
├── handlers.rs       (unchanged — CEF trait wrappers)
├── helpers.rs        (unchanged)
├── wndproc.rs        (unchanged — Windows only)
└── tests.rs          (the 104-line #[cfg(test)] block, or keep inline in the file
                       whose functions it exercises — recovery_pages tests → recovery_pages.rs)
```

## Execution notes

- Each new file adds `impl AgentMuxHandler { ... }` with just its methods — legal since inherent impls compose across files within a crate. The struct definition + fields stay in `mod.rs`; **all fields the moved methods touch must be reachable** — if any field is currently private-to-module and a method in another file uses it, fields on a struct defined in `mod.rs` are visible to sibling submodules only if `pub(crate)`/`pub(super)` OR if the impl is in the same module. **Key rule:** methods in `lifecycle.rs` etc. can access private fields of `AgentMuxHandler` ONLY if those files are child modules of the module that defines the struct — which they are (all under `client`), BUT Rust field privacy is module-scoped: a struct defined in `client::mod` has fields private to `client` module itself, and child modules (`client::lifecycle`) can access them via `pub(super)` visibility or if fields are `pub(crate)`. **Verify with `cargo check`;** if field-access errors appear, mark the relevant fields `pub(crate)` (they're on a crate-internal struct, so this is safe and non-breaking).
- The 5 tests mostly cover recovery pages / memory budgets / URL matching — move them next to the code they test (`recovery_pages.rs`, `crash_recovery.rs`) so `use super::*` resolves.
- Preserve every `#[cfg(target_os = ...)]` guard exactly.
- No `#![allow(unused_imports)]`; trim per file.

## Verification gate

- `cargo check -p agentmux-cef` clean on Windows, zero new warnings
- The 5 unit tests pass
- Manual re-read of moved `#[cfg]` blocks
- reagent review

## Risk: **Medium.** Cleaner than shell.rs (inherent impls split freely), but watch struct field visibility across the new child modules — bump fields to `pub(crate)` if `cargo check` complains (safe on a crate-internal type).
