# SPEC: Browser Pane Code Modularization

Status: draft
Date: 2026-04-18
Owner: AgentA
Motivation: `browser_panes.rs` is 475 lines with four distinct concerns
(lifecycle state, CEF client integration, Win32 HWND ops, UI-thread task),
and it tangles with `client.rs` via a true cycle. The pane-related code
has already been edited 4 times in 48 hours; every edit threads through
the same tight coupling. Modularization pays for itself if it makes the
next edit isolated.

## 1. Current dependency graph (post-#432 main)

Actual edges observed via grep, not the static-analysis false positives:

```
ipc.rs       ──────────────► state.rs
                                │
                                ├─ browsers: HashMap<Label, Browser>
                                ├─ browser_panes: BrowserPaneManager
                                ├─ window_meta, window_id_map, …
                                └─ pending_window_labels

browser_panes.rs ─► state::AppState  (input to every method)
browser_panes.rs ─► client::{AgentMuxHandler, AgentMuxClient, ALLOW_PANE_FOCUS_ONCE}
browser_panes.rs ─► commands::window::find_own_top_level_window

client.rs        ─► state::{AppState, WindowKind}
client.rs        ─► state.browser_panes.drain_closed_label(lbl)   ◄── CYCLE
                                                                  (through AppState field)

commands/window ─► client::dlog                   (one-way, utility ref)
commands/window ─► ui_tasks, state, events        (one-way, normal)

commands/providers ─► state::AppState            (one-way)
```

### True cycles

Exactly **one** real cycle exists:

```
browser_panes.rs ◄──► client.rs
```

- `browser_panes.rs` imports `client::AgentMuxHandler` (builds the pane's
  CEF client inside `CreatePaneTask`) and `client::ALLOW_PANE_FOCUS_ONCE`
  (the focus-redirect bypass flag used by `BrowserPaneManager::focus`).
- `client.rs` calls `self.state.browser_panes.drain_closed_label(lbl)` in
  `on_before_close` so a CEF-initiated pane teardown can clean up the
  lifecycle map.

Everything else the user flagged as "browser_panes ↔ window / state /
providers / ipc" is one-way composition through `AppState` — that's the
normal pattern for a shared state object and not actually cyclic at the
module-import level. Worth tidying but not the core issue.

### Why this cycle matters

- The cycle forces every edit to `BrowserPaneManager` to reach into
  `client.rs` internals (pane-handler construction, focus-redirect flag).
- Pane-specific callback logic lives in `client.rs` (the `on_before_close`
  pane branch, `install_pane_focus_redirect`, `ALLOW_PANE_FOCUS_ONCE`), so
  reading "how panes work" means opening both files.
- The last four lifecycle bugs each needed edits to both files.

## 2. Concerns tangled in `browser_panes.rs` today

1. **Lifecycle state** — `PaneEntry { label, state: Live|Closing }`,
   `PANE_LABEL_SEQ`, the `panes` mutex-map, `drain_closed_label`. Pure
   data structures; no CEF, no Win32.
2. **CEF browser ops** — `close()`'s refcount drop, `navigate`'s
   `load_url`, `go_back/forward/reload`, `defocus_all`, `focus`.
3. **Win32 HWND ops** — `DestroyWindow`, `SetWindowPos`, `SetFocus`,
   `notify_move_or_resize_started`. Pure OS, Windows-only.
4. **UI-thread pane creation** — `CreatePaneTask`: `find_own_top_level_window`,
   `browser_host_create_browser`, `WindowInfo::set_as_child`, the
   pane client construction via `AgentMuxHandler::new_with_pane`.

Four concerns, one file. The `PaneCloseOps` trait I just introduced is the
first step toward splitting concern #2 from concerns #3 and #4 — this spec
proposes the rest.

## 3. Proposed layout

```
agentmux-cef/src/pane/
├── mod.rs              — re-exports the public surface
├── lifecycle.rs        — PaneEntry, PaneLifecycle, PANE_LABEL_SEQ,
│                         the state-machine methods (register_live,
│                         mark_closing, drain, is_closing, …).
│                         NO CEF, NO Win32. Fully unit-testable today.
├── manager.rs          — BrowserPaneManager: the orchestration layer.
│                         Threads state machine + ops + hwnd calls.
│                         Re-exports the public create/close/navigate/… API
│                         that ipc.rs calls today.
├── ops.rs              — PaneCloseOps trait + AppStateCloseOps production
│                         impl (extend to PaneNavigateOps, PaneFocusOps
│                         as ops grow tests).
├── hwnd.rs             — #[cfg(target_os = "windows")] helpers:
│                         destroy_hwnd(), set_focus_hwnd(),
│                         reposition_hwnd(), install_pane_focus_redirect(),
│                         ALLOW_PANE_FOCUS_ONCE static, PANE_WNDPROCS map.
│                         Moves from client.rs:1000-1138.
├── creation.rs         — CreatePaneTask. Still needs to import
│                         AgentMuxHandler + AgentMuxClient from client —
│                         that stays as a one-way dep, not a cycle
│                         (manager.rs no longer needs client for this).
└── callbacks.rs        — Pane-specific CEF callback bodies that today
                          live inline in client.rs:
                            • on_after_created pane branch (z-order raise)
                            • on_before_close pane branch (drain_closed_label)
                            • on_set_focus pane cancel
                            • on_load_end pane skip-IPC
                          Exposed as free functions `client.rs` calls.
```

Old `browser_panes.rs` becomes a thin re-export shim for one release
(`pub use crate::pane::*;`) so external call sites (`ipc.rs`,
`state.rs`, `client.rs`) don't need to change in the same PR. Delete the
shim in a follow-up.

## 4. Breaking the cycle

The cycle exists because `client.rs::on_before_close` calls
`state.browser_panes.drain_closed_label()`. After the split:

- `pane::callbacks::on_before_close_pane(state, browser)` becomes the
  function `client.rs` calls for the pane branch.
- The function uses `state.browser_panes.drain_closed_label()` — a
  one-way import from `pane/callbacks.rs` → `state::AppState` →
  `pane::manager::BrowserPaneManager`. No edge from manager → client.
- `client.rs` keeps importing `pane::callbacks`. No edge back.

Result:

```
client.rs ──► pane::{callbacks, creation}
pane/     ──► state::AppState
state.rs  ──► pane::BrowserPaneManager  (composition, one-way)
```

No cycle.

Similarly, `ALLOW_PANE_FOCUS_ONCE` + `install_pane_focus_redirect` move
from `client.rs` into `pane::hwnd`. `client.rs` no longer owns any
pane-specific statics; the `on_set_focus` pane branch in `client.rs`
just calls `pane::callbacks::on_set_focus_pane(...)` which knows about
the flag.

## 5. Testability gains

Once the split lands:

- **`pane::lifecycle`** — every test I wrote in #431 lives here natively,
  no `#[cfg(test)]` helpers needed.
- **`pane::ops`** — the `PaneCloseOps` pattern from #431 becomes the
  default shape. Add `PaneNavigateOps`, `PaneFocusOps`, `PaneResizeOps` as
  needed. Tests target `manager::close_with`, `manager::focus_with`, etc.
- **`pane::callbacks`** — pure functions taking `&Arc<AppState>` and a
  small event enum. Easy to drive from integration tests with a
  `#[cfg(test)]` AppState that uses a mock `browser_panes` manager.
- **`pane::hwnd`** — can't unit-test Win32 calls, but moves them into one
  file so "touches OS" is visible at a glance. Integration tests (spec
  `SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md` L4) cover these.

## 6. Phased delivery

Don't do this as one PR — the diff would hide bugs. Four phases, each
mergeable independently:

### Phase 1 — lift state machine (no behavior change)
- New `pane/lifecycle.rs` with `PaneEntry`, `PaneLifecycle`,
  `PANE_LABEL_SEQ`, `PaneStateMachine` struct.
- `BrowserPaneManager` holds `state: PaneStateMachine` and delegates the
  pure-state parts. Public API unchanged.
- Move `browser_panes/tests` into `pane/lifecycle/tests`.
- Risk: very low. Pure code motion.

### Phase 2 — lift hwnd helpers
- New `pane/hwnd.rs`. Move `ALLOW_PANE_FOCUS_ONCE`, `PANE_WNDPROCS`,
  `install_pane_focus_redirect`, `enum_children` helper, the direct
  Win32 calls from `browser_panes.rs::{resize, focus}` and
  `browser_panes.rs::close`.
- `client.rs` keeps using `ALLOW_PANE_FOCUS_ONCE` via re-export until
  phase 4 moves the set-focus callback.
- Risk: medium. Pure motion but touches every Win32 call site.

### Phase 3 — lift CreatePaneTask
- New `pane/creation.rs`. Contains `CreatePaneTask` + the call to
  `AgentMuxHandler::new_with_pane` + the `browser_host_create_browser`
  call.
- Still depends on `client.rs` (for the handler type) — but only
  one-way. No cycle.
- Risk: low. Self-contained chunk.

### Phase 4 — lift pane callbacks, close the cycle
- New `pane/callbacks.rs` with `on_before_close_pane`, `on_after_created_pane`,
  `on_set_focus_pane`, `on_load_end_pane`.
- `client.rs` drops the `if self.is_pane` branches and the inline pane
  logic; the branches become one-liners that call into `pane::callbacks`.
- Drop the `ALLOW_PANE_FOCUS_ONCE` re-export from `client.rs`.
- **Cycle broken here.** `client.rs` has no direct code dependency on
  `browser_panes` — only on `pane::callbacks` and `pane::creation`.
- Risk: medium. Subclass-install timing is fiddly; land L3 tests from
  `SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md` §5.1 before this phase so
  regressions are caught.

## 7. What not to do

- **Don't introduce a new shared state type** (`PaneContext`, etc.) —
  just move code around `AppState`. New abstractions invite new call
  sites.
- **Don't merge `state.browsers` and `state.window_meta` rewrites into
  this work.** Those are their own cleanup. Stay focused on the pane
  split.
- **Don't delete the old file names in phase 1.** Leave `browser_panes.rs`
  as a re-export shim until phase 4 is merged. Minimizes merge noise on
  every in-flight PR.

## 8. Done criteria

- `grep -l "pub use crate::pane::" src/browser_panes.rs` is the only
  match — the original file is a stub.
- `client.rs` has no references to `browser_panes`, only to
  `pane::callbacks` / `pane::creation`.
- `cargo depgraph --no-externals | grep -c "pane -> client"` = 0.
- L1+L2+L3 tests from `SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md` all green.
- Next lifecycle bug (there will be one) can be root-caused by reading
  only `pane/`, without opening `client.rs`.

## 9. Not in scope

- Modularizing `client.rs` itself (multi-window, drag, Subwindow taskbar
  grouping logic is intertwined with lifecycle logic). Separate spec.
- Changing `AppState` ownership. `state.rs` grows; that's a different
  conversation about when to split into `BrowserState` / `PaneState` /
  `WindowState`.
- Moving `commands/window.rs` into a `window/` module. Similar smell,
  separate follow-up.
