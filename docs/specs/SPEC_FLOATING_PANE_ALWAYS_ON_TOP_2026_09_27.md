# SPEC: "Always on top" for floating panes

**Date:** 2026-09-27
**Status:** active — Phase 1 (Windows) implemented in PR #3970; live verification pending on a test machine (#3969). Phase 2 (macOS/Linux, §7) not started.
**Author:** Clamk, at the repo owner's request: "introduce 'always on top' for floating panes … an extended header entry near the window controls (similar to the stash on the agent pane), use the tack symbol; when selected it is highlighted using the theme color and the floating pane will remain atop every window in the running agentmux instance. If 2 tacked floating panes overlap, they behave normally relative to each other."
**Scope:** Windows first (Phase 1). macOS/Linux in Phase 2.

---

## 1. The ask, made precise

A floating pane (a pane torn off into its own native window) gets a **tack** toggle
in its header. While tacked:

1. The floater stays **above every window of this AgentMux instance**: the main
   window, other top-level AgentMux windows (`window-*`, promoted `window-pool-*`,
   tear-offs), and untacked floaters. This holds even when one of those is clicked
   or activated.
2. Two tacked floaters are **ordinary peers of each other**: clicking one brings it
   in front of the other, as normal windows do.
3. It is **not** above other applications. Switching to another app lets that app
   cover the floater, as today. "Always on top" means within AgentMux. See §3 for
   why global topmost is rejected.
4. It is **not** above another AgentMux instance (a separate process, e.g. a dev
   build next to the installed app). "The running instance" is this process only.
5. The tack is a **per-floater** setting, remembered across a page reload of the
   floater. It is dropped when the pane is redocked (§6.3).

Untacked floaters behave exactly as today.

## 2. How floaters work today (investigated 2026-09-27, `main` @ `f1cfceb02`)

### 2.1 Host (Windows)

- A floater is a raw Win32 popup, not a CEF Views window. `create_popup`
  (`agentmux-cef/src/floating_pane.rs:713-858`) calls `CreateWindowExW(WS_EX_TOOLWINDOW,
  …, WS_POPUP | WS_THICKFRAME, …, hWndParent = null)`. There is **no owner**
  (`:814`, "no owner — see module doc") and no `WS_EX_TOPMOST`. The CEF browser is
  embedded with `set_as_child`.
- **z-order today: none.** Floaters are ordinary peers of every other window, ours
  and other apps'. Clicking the main window covers them. History:
  - The original design owned floaters by main (`SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md`),
    which pinned them permanently above main (#1560).
  - #1574 (`cf71cd06`) removed the owner and pushed floaters below main on
    `WM_ACTIVATE`.
  - #1677 (`f33cd94cb`, "floater independence") deleted all z-order handling.
    `floater_cascade_wndproc` (`client/wndproc.rs:145-193`) is now a pass-through.
  - Several comments still describe the removed behaviour and should be corrected
    in passing: `floating_pane.rs:16-36,44`, `client/lifecycle.rs:365-369`,
    `SPEC_REDOCK_FRAMEWORK_HARDENING_2026_07_27.md:6-10`.
- `floating_pane_wndproc` (`floating_pane.rs:580-704`) handles only
  `WM_NCCALCSIZE`, `WM_NCACTIVATE`, `WM_NCHITTEST`, `WM_DESTROY` and `WM_SIZE`.
- Registries: `AppState.window_hwnds[label]` holds the outer popup HWND (`:223-226`).
  `floater_hwnd_for_label` resolves it. `ACTIVE_FLOATER_HWNDS` exists for debug
  snapshots (`:78-124`).
- Floaters come from a pool: `spawn_pane_pool_window` / `promote_pane_pool_window`
  (`commands/pane_pool.rs:222-557`). A promote calls `SetWindowPos(HWND_TOP)` and
  `SW_SHOWNORMAL`, then relabels `floating-pool-*` → `floating-*` (`RelabelBrowser`
  re-keys `pane_window_states`). Pool HWNDs are always fresh and floaters are
  never demoted, so no stale topmost state can come in from the pool.
- Every move/resize path passes `SWP_NOZORDER` (`commands/window/motion.rs`,
  `ui_tasks/drag.rs`, `toggle_floating_maximize` in `commands/window/chrome.rs:239-248`).
  The opacity code preserves `GWL_EXSTYLE`. A topmost state survives all of them.
- The WRR hooks ignore floaters: their window class isn't an app class
  (`wrr/classify.rs`, `wrr/win_event.rs:960-1008`). The quit watchdog excludes them
  by class and `BrowserKind::Floater`. The launcher tracks no z-order. **A topmost
  change touches none of this.**
- Redock hit-testing skips `floating-*` windows (`commands/window/motion.rs:448`), so
  a tacked floater over the main window can't hide main from redock.
- The only existing "always on top" helper is `post_set_always_on_top`
  (`ui_tasks/pool.rs:499-546`, CEF Views `set_always_on_top(1)`, used by the tray
  panel). It resolves through `get_window_on_ui`, which returns `None` for a
  Windows floater, so it can't be reused on Windows. It also has no "off" path.

### 2.2 Host (macOS / Linux)

Floaters are ordinary frameless CEF Views top-level windows
(`commands/floating_pane.rs:268-373`, `WindowKind::FullInstance`), with no owner or
level. CEF Views `set_always_on_top(bool)` applies directly.

### 2.3 Frontend

- A floater renders `FloatingPaneWorkspace` (`frontend/app/workspace/floating-pane-workspace.tsx`),
  chosen by `IS_FLOATING_PANE` (`app.tsx:387`). The pane's standard
  `BlockFrame_Header` is the only chrome. The window label comes from
  `?windowLabel=floating-*`, and the pane's block is the tab's single block.
- The header's right end is `EndIcons` (`frontend/app/block/blockframe.tsx:306-418`):
  - the view's `endIconButtons` (the agent pane's **Stash** is one, `agent-model.ts:209-223`);
  - a separator;
  - (mic);
  - minimize (only with more than one leaf);
  - magnify, or `FloatingMaximizeButton` in a floater (the `<Show when={floatingLabel()}>`
    branch, `:397-406`);
  - close.
- Stash is a `toggleiconbutton`: `ToggleIconButton` (`element/iconbutton.tsx:45-74`)
  sets `title` / `aria-label` ("Stash (Active)" when on) and the `toggle` / `active`
  classes. `iconbutton.scss:40-47` colors `.toggle.active` with
  **`var(--accent-color)`**, the theme color, defined per theme in `themes/*.scss`.
- Icons are Font Awesome 6.7.2 Pro via `makeIconClass`. **`fa-thumbtack`** and
  `fa-thumbtack-slash` are available (`public/fontawesome/css/fontawesome.min.css`).
  Nothing uses them today.
- Host calls go through `getApi().windows` (`WindowHostApi`, `types/custom.d.ts:186-221`),
  implemented in `app/host/cef-host-commands.ts:89-128`. The host-API seam
  (`SPEC_HOST_API_SEAM_2026_09_26.md`, enforced by `app/host/host-boundary.test.ts`)
  forbids `invokeCommand` outside the seam files. A platform-specific UI is gated
  with a `HostCaps` capability and `hostHas(...)`.
- Floater meta uses the `pane:floating_*` prefix: `pane:floating_placement` and
  `pane:floating_normal_rect`, written by the host in `toggle_floating_maximize`
  via `client::backend_update_block_meta`. Nothing reads them back when a floater
  is created. Tear-off and redock **move** the same block (`sagas/tear_off_block.rs`,
  `sagas/redock_floating_pane.rs`), so block meta travels with the pane. "Split"
  copies all of the source block's meta (`block/pane-actions.ts:46-61`).
- "Pin" already means several other things (pinned tabs, widget-bar pins,
  transcript pins, scroll pin-to-bottom, CLI version pins, MOS pinning). This feature
  is called **"Always on top"** in text and code, not "pin". The icon is the tack.

## 3. Mechanism (Windows)

The native "keep above" primitive, `HWND_TOPMOST`, is **global**: a topmost window
is above every non-topmost window on the desktop, including other apps. The
tear-off spec rejected that as "user-irritating"
(`SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` §10), and the owner scoped this feature
to "every window in the running agentmux instance". Two alternatives, and the
choice:

| option | how | verdict |
|---|---|---|
| A. Ownership | make the floater owned by main | **Rejected.** One owner only: it stays above main but not above `window-*` windows. It also brings back #1560's permanent pinning and the owner minimize/destroy cascade #1677 removed. |
| B. Re-raise on activation | on every activation of one of our windows, re-raise tacked floaters with `SetWindowPos(HWND_TOP)` (#1574's approach) | **Rejected.** It needs hooks on every top-level (the WRR hook filters floaters out), it flickers (the activated window paints on top first), and it races drags. |
| **C. Topmost while we're the active app** | `HWND_TOPMOST` while this process is the foreground app; drop out of the topmost band when another app is activated | **Chosen.** One message, a stable z-band, no flicker. Tacked-vs-tacked is plain topmost-band behaviour, which gives §1.2 for free. |

**C in detail.** Each floater's `floating_pane_wndproc` handles `WM_ACTIVATEAPP`,
which Windows sends to every top-level window of a thread when activation moves
to or from another process:

- `wParam == TRUE` (we became the active app) and the floater is tacked →
  `SetWindowPos(hwnd, HWND_TOPMOST, 0,0,0,0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)`.
- `wParam == FALSE` (another app, or another AgentMux instance, became active) and
  tacked → take the floater out of the topmost band **directly below the newly
  active window**, `SetWindowPos(hwnd, GetForegroundWindow(), 0,0,0,0, SWP_NOMOVE |
  SWP_NOSIZE | SWP_NOACTIVATE)`. Placing a topmost window after a non-topmost one
  clears its topmost state. The simpler `HWND_NOTOPMOST` puts it at the top of the
  normal band, which could leave it *above* the app the user just switched to. If
  `GetForegroundWindow()` is null or is the floater itself, fall back to
  `HWND_NOTOPMOST`.
- Toggling the tack while we're active applies `HWND_TOPMOST` / the `wParam == FALSE`
  placement immediately. Untacking while active uses `HWND_NOTOPMOST`: it stays
  where it is visually, but the next activation of another AgentMux window can
  cover it, which is normal behaviour.

The decision is a pure function, unit-tested apart from Win32:

```rust
enum ZAction { None, MakeTopmost, DropBelowForeground, DropToNormal }
fn z_action(tacked: bool, app_active: bool, event: ZEvent /* AppActivated | AppDeactivated | TackToggled */) -> ZAction
```

Consequences to accept or handle:

1. **Our own non-topmost windows can be covered by a tacked floater**, which is the
   point of the feature. That also covers our **transient windows**, which need
   exceptions:
   - **Credential-approval and memory-adoption approval windows**
     (`initialView=credential-approval` / `memory-adoption-approval`) must never
     be hidden behind a floater. They are security prompts. When one opens, make
     it topmost as well, so it sits above the floaters. Required, Phase 1.
     Topmost windows stack by the most recent assertion, so making it topmost
     once isn't enough: switching apps and back re-asserts the floater's topmost
     state and would put it in front. Open approval windows are therefore kept
     in a registry, and every time a floater (re)enters the topmost band they
     are re-raised over it (ReAgent P1 on #3970).
   - **Native file pickers** (`rfd::FileDialog`, `commands/platform.rs:1058-1190`)
     run on a worker thread with no owner, so a tacked floater can cover them.
     Phase 1: while a host file dialog is open, temporarily move tacked floaters to
     the normal band (`DropToNormal`) and restore them when it closes. The dialog
     calls are all in one module, so a guard object around `pick_file` /
     `pick_folder` / `save_file` covers them.
   - **The tray panel** is already `WS_EX_TOPMOST` (Views `set_always_on_top`). Both
     are topmost, so the most recently activated is in front, which is fine.
   - **The snap-preview overlay** (`ui_tasks/snap_preview.rs`) re-asserts
     `HWND_TOPMOST` on each show, so it stays above. No action.
2. **Main minimized**: floaters have been independent since #1677, so a tacked
   floater stays visible while main is minimized, like an untacked one. No change.
3. **Minimizing a tacked floater**: minimize/restore keeps `WS_EX_TOPMOST`. No action.
4. **Multi-monitor**: `SPEC_FLOATING_PANE_MULTI_MONITOR_TASKBAR_2026_07_27` (not
   built) would reuse `set_taskbar_hidden`, which hides, restyles and re-shows the
   window. If that lands, it must re-apply the z-state afterwards. Noted there.

## 4. UI

- **Placement:** in `EndIcons`' floater-only branch (`blockframe.tsx:397-406`),
  **immediately before `FloatingMaximizeButton`**, so it sits with the window
  controls: `[view buttons (e.g. Stash)] | [separator] | [mic] | tack | maximize |
  close`. It shows only in floaters (`floatingLabel()` non-null) and only when the
  host supports it (`hostHas("floatingAlwaysOnTop")`).
- **Element:** a `ToggleIconButton` like Stash:
  `{ elemtype: "toggleiconbutton", icon: "thumbtack", title: "Always on top", active }`.
  The tooltip and aria-label read "Always on top", or "Always on top (Active)" when
  on, via the component. When active, the existing `.toggle.active` rule colors it
  `var(--accent-color)` (the theme color), as the owner asked.
- **Contrast:** a pane with a custom header color (`frame:hue`,
  `blockframe.tsx:679-715`) can make an accent-colored icon hard to see. Apply the
  same rule the header already uses for its text (`pickReadableTextColor`), and add
  `background: var(--highlight-bg-color)` to the active tack so the on-state is
  visible whatever the header color. Keep it scoped to the tack, because changing
  `.toggle.active` globally would also restyle Stash and the search toggles.
- **Header drag:** the drag handler already skips `button` / `[role='button']`
  (`floating-pane-workspace.tsx:141-143`), so clicking the tack doesn't start a
  window drag.
- No keyboard shortcut or context-menu entry in Phase 1. A "Always on top" item in
  the floater's pane context menu (`pane-actions.ts`) is a cheap follow-up.

## 5. Host command and state

- **Command:** `set_floating_always_on_top { label, block_id, on }` → `{ on }`,
  routed in `agentmux-cef/src/ipc.rs` next to `toggle_floating_maximize`, handler in
  `commands/window/chrome.rs`. It follows `toggle_floating_maximize`'s shape:
  1. a pure reducer command `HostCommand::SetFloatingAlwaysOnTop { label, on }`
     (`reducer/mod.rs`, handler in `reducer/pane_window.rs`) that sets a new
     `PaneWindowState.always_on_top: bool` (`state/browser_pane.rs:101-107`);
  2. then the Win32 side effect on `floater_hwnd_for_label(label)`, using `z_action`
     with `TackToggled` and the current app-active state (`GetForegroundWindow()`'s
     process == ours);
  3. then write the block meta in the background, like the placement meta
     (`client::backend_update_block_meta`).
- `floating_pane_wndproc`'s `WM_ACTIVATEAPP` handler reads the floater's tack state
  from `PaneWindowState` via its label (the wndproc already has label/HWND
  bookkeeping through `ACTIVE_FLOATER_HWNDS`). Keep the lookup cheap and lock-free
  on the UI thread: an `AtomicBool` per floater in the floater registry, mirrored
  from the reducer, is enough.
- **Pool:** a promote re-keys `pane_window_states` (`RelabelBrowser`), so the flag
  follows. `EvictFloatingPaneWindowState` drops it on close. The OS topmost bit
  dies with the HWND. Nothing else to reset, because floaters are never demoted
  back into the pool.
- **Frontend API:** `WindowHostApi.setFloatingAlwaysOnTop(label: string, blockId: string,
  on: boolean): Promise<boolean>` in `types/custom.d.ts`, implemented in
  `cef-host-commands.ts`. `HostCaps.floatingAlwaysOnTop: boolean`: true on Windows
  (Phase 1), false elsewhere until Phase 2, and false in the test host by default.

## 6. Persistence

### 6.1 Key
`pane:floating_ontop: boolean` on the floater's block (`floatingPaneId`), in the
existing `pane:floating_*` family. Add it to `MetaType` in
`frontend/types/srv-types.d.ts`. `pane:floating_placement` isn't declared there
either, so declare both.

### 6.2 Re-apply
Nothing reads floater meta back at creation today (§2.3). Rather than teach the
host to read block meta at window creation, the **frontend** re-applies it:
`FloatingPaneWorkspace`, once its block has loaded, calls
`setFloatingAlwaysOnTop(label, blockId, true)` if `pane:floating_ontop` is true.
That covers a floater page reload, a pool promote (fresh HWND, then mount), and
(when floaters are restored after a restart, which isn't built yet) that too, with
no extra host path.

### 6.3 Lifetime
- **Redock clears it.** `RedockFloatingPane` sets `pane:floating_ontop: null` in
  the same saga. A later tear-off starts untacked; re-tacking is one click, and a
  pane unexpectedly popping over everything is worse.
- **Split drops it.** Exclude `pane:floating_ontop` in `pane-actions.ts`' meta copy,
  generically, not per view: the new pane is docked, and the key means nothing
  there.
- **Tear-off of an already-tacked block** can't happen, because redock cleared it.
  If the key is still present anyway, e.g. after a crash mid-redock, §6.2 re-applies
  it, which is acceptable.

## 7. Phase 2: macOS and Linux

- Floaters there are CEF Views windows, so use Views `set_always_on_top(bool)`: an
  `on`/`off` variant of `post_set_always_on_top`, which today only turns it on.
- App-scoping: on macOS, handle `NSApplicationDidResignActiveNotification` /
  `DidBecomeActive` to toggle the window level (`NSFloatingWindowLevel` ↔
  `NSNormalWindowLevel`). On Linux the keep-above hint (`_NET_WM_STATE_ABOVE`) is
  advisory and window-manager dependent; ship it global with a note, or leave
  Linux on `hostHas` = false.
- Flip `HostCaps.floatingAlwaysOnTop` per platform as each lands.

## 8. Tests

This machine is not used for live testing. Unit tests go in the PR; the live checks
go in a GitHub issue for the test machine.

**Unit (Phase 1 PR):**
- Rust: `z_action` truth table; reducer `SetFloatingAlwaysOnTop` sets/clears
  `PaneWindowState.always_on_top` and survives `RelabelBrowser`; eviction drops it.
- Frontend: the tack renders only in a floater with the capability; `toggle` /
  `active` classes and "Always on top (Active)" title follow `pane:floating_ontop`;
  a click calls `windows.setFloatingAlwaysOnTop(label, blockId, !on)`
  (`makeTestHostApi`); `FloatingPaneWorkspace` re-applies a stored `true` on mount;
  split drops the key; host-boundary test still passes.

**Live (issue, test machine, Windows):**
1. Tack a floater; click main, a `window-*` window, and another floater: the tacked
   floater stays in front of all of them.
2. Two tacked floaters overlapping: clicking each brings it in front of the other.
3. Alt-Tab to another app: that app covers the tacked floater. Switch back: the
   floater is on top again. Same with a second AgentMux instance.
4. The tack icon is in the theme color when on, readable on a colored header, and
   the tooltip reads "(Active)".
5. Open a host file picker from main while a tacked floater overlaps it: the picker
   is fully usable, and the floater is tacked again after it closes.
6. Trigger a credential-approval window under a tacked floater: the approval window
   is on top.
7. Reload the floater page: still tacked. Redock and tear off again: untacked.
8. Maximize/restore, minimize/restore, drag across monitors: tack state holds.

## 9. Decisions (owner, 2026-09-27)

1. **Redock forgets the tack** (§6.3): yes. A later tear-off starts untacked.
2. **Not above other applications** (§1.3): confirmed. Scoped to this instance.
3. **Phase 1 Windows-only**, button hidden on macOS/Linux until Phase 2: yes.
