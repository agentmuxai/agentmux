# SPEC: Restore keyboard focus to the active pane on window re-activation (Windows)

**Date:** 2026-05-23
**Author:** AgentX
**Status:** Draft

---

## TL;DR

On Windows, after the user clicks away from AgentMux to another app and returns, the terminal pane sometimes accepts no keyboard input until they "wake" focus by typing into a sibling AgentMux window. The root cause is a missing link in the focus chain: **no layer reacts to the top-level window regaining OS focus by re-establishing keyboard focus on the active pane**. The OS hands focus to the top-level HWND; nobody routes it down to the CEF render-widget child HWND or the xterm hidden textarea, so keystrokes land on `document`/`body` and are dropped.

The fix wires two missing handlers — a Rust `WM_ACTIVATE`/`WM_SETFOCUS` handler on the host window that delegates focus to the last-active pane's CEF child HWND, and a frontend `window`-level `focus` listener that calls `giveFocus()` on the active terminal as a belt-and-braces. Together they make focus-on-reactivate deterministic.

---

## 1. The behavior we want

When the user switches away from AgentMux and back (Alt+Tab, clicking the taskbar, clicking the title bar, switching from another window):

1. The top-level AgentMux HWND becomes active.
2. Keyboard focus deterministically lands on **the pane that was active before the switch-away** — i.e. typing immediately reaches the xterm textarea / agent input / editor that the user was using.
3. No "first keystroke is dropped" frame, no "type in a sibling window first to unlock," no need to click the pane.

This must hold for **terminal** panes, **agent** panes, and **editor** panes equally. Browser panes are out of scope (their CEF child window manages its own input).

---

## 2. The focus chain today

On Windows, keyboard focus crosses four layers. Each layer has independent state:

| Layer | Owner | What "focused" means |
|---|---|---|
| **OS / Win32** | Top-level HWND | `WM_ACTIVATE wParam ≠ 0`; receives `WM_SETFOCUS` |
| **CEF child HWND** | `Chrome_RenderWidgetHostHWND` per browser | Win32 `SetFocus(target)` claimed |
| **Chromium / CEF** | `cef_browser_host_t::set_focus(true)` | Chromium routes keystrokes to its DOM |
| **DOM / xterm** | `HTMLTextAreaElement.focus()` | xterm hidden textarea reads keystrokes |

A keystroke reaches the terminal only when **all four** are aligned: OS active → correct child HWND has SetFocus → Chromium browser host has set_focus(true) → xterm textarea is the DOM `activeElement`.

### What is wired today

- **Pane creation (`agentmux-cef/src/browser_pane/callbacks.rs:35-75`):** When a browser pane HWND is created, `install_browser_pane_focus_redirect()` is installed as a subclass — on `WM_SETFOCUS`, it forwards focus to `GetAncestor(hwnd, GA_ROOT)` *unless* the `ALLOW_BROWSER_PANE_FOCUS_ONCE` flag was set by a deliberate caller (`agentmux-cef/src/browser_pane/hwnd.rs:141-310`, esp. `:249-277`). Rationale: prevent pane HWNDs from stealing focus during navigation; `WM_MOUSEWHEEL` still routes correctly without the steal.
- **Pane → Main focus IPC (`agentmux-cef/src/ui_tasks.rs`, `MainFocusReclaimTask::execute()`, ~`:100-180`):** When the frontend fires the `main_window_focus` command (`frontend/app/block/block.tsx:200-210`), the host calls `browser.host().set_focus(1)` on the main browser, walks the HWND tree via `find_main_render_widget()` to find `Chrome_RenderWidgetHostHWND`, calls `SetFocus(target)`, and calls `defocus_all()` on panes. **This path triggers only on DOM-input focus events inside main**, not on OS window re-activation.
- **xterm focus method (`frontend/app/view/term/termViewModel.ts:375-388`):** `giveFocus()` calls `this.termRef.current.terminal.focus()` on xterm.js. It is **only called** when the in-pane search overlay closes (`frontend/app/view/term/term.tsx:142`).
- **Active-pane signal (`frontend/app/store/focusManager.ts`, `nodeModel.isFocused()`):** The frontend knows which pane is the active one.

### What is missing

- **No top-level `WM_ACTIVATE` handler** in the host window proc. When Windows reactivates the AgentMux HWND, no AgentMux code runs.
- **No `WM_KILLFOCUS` tracking** on browser-pane HWNDs — the host has no record of "which child HWND last held focus."
- **No frontend `window.addEventListener("focus", …)` listener.** `AppFocusHandler` in `frontend/app/app.tsx:195-230` exists but **early-returns `null`** — it is a debug-only stub.
- **No connection between `nodeModel.isFocused()` and an `OnWindowReactivate` event** — even though the frontend knows the active pane, nothing fires `giveFocus()` on window-level focus.

The focus chain is fully wired for *intra-app* focus transitions (pane → main, search close → terminal). It is **not wired** for *cross-app* re-activation. That is the entire bug.

---

## 3. Root cause

When the user Alt+Tabs back to AgentMux:

1. Windows sends `WM_ACTIVATE(WA_ACTIVE)` then `WM_SETFOCUS` to the **top-level host HWND**.
2. Default `DefWindowProc` handling does not propagate focus to a specific child HWND — it just marks the window active.
3. The CEF `Chrome_RenderWidgetHostHWND` that owned focus before the switch-away is **not re-`SetFocus`-ed**. Chromium does not know it is the keyboard target.
4. JavaScript keydown events therefore fire on `window`/`document` with `activeElement === document.body` (or a stale element that no longer has child focus). xterm's hidden textarea is not the target → keystrokes are silently dropped.
5. The user clicks into a pane → that click triggers `WM_LBUTTONDOWN` on the child HWND → Windows calls `SetFocus` on the click target → the chain repairs itself → typing works again.

The xterm pane's `giveFocus()` mechanism *exists* and *works*; the bug is that nothing **invokes** it on window re-activation. Likewise, `MainFocusReclaimTask` *exists* and *works* for main DOM inputs; nothing **invokes** an equivalent reclaim for the active pane on window re-activation.

---

## 4. Why it is intermittent — and why the workaround works

### Why intermittent

The bug fires only when **the pre-deactivation focus owner was a CEF child HWND that loses its `HasFocus` state during deactivation in a way that does not auto-recover**. Several factors stochastically determine recovery:

- Which child HWND last had focus (main vs. pane vs. terminal) before switch-away.
- Whether the user clicked the title bar / taskbar (Windows often re-`SetFocus`-es the prior child) versus Alt+Tabbed (Windows just re-activates the top HWND).
- Whether any IPC traffic between the host and srv caused a transient HWND focus claim during the switch-away window.
- Whether CEF's internal focus-tracking observers happened to refire (race-dependent).

When any of those incidentally re-`SetFocus`-es the right child, the user perceives "it just works." When none does, the keystrokes drop.

### Why typing in a sibling AgentMux window fixes it

Each AgentMux instance is a **separate process tree** under its own Job Object (`agentmux-launcher/src/main.rs:325-351`, `KILL_ON_JOB_CLOSE`). Activating a sibling window:

1. Sends `WM_ACTIVATE` to the sibling's top-level HWND.
2. Sibling Chromium reinitializes its focus tracking inside that process.
3. Returning to the original window with Alt+Tab causes Windows to re-`SetFocus` the **previous child HWND that Windows recorded as focused at deactivation time**.
4. After the sibling's keystroke cycle, that "previously focused child" pointer has been refreshed by Windows to a valid target, and the original window's child gets `SetFocus` correctly on re-entry.

This is a Windows focus-chain side effect, not a designed recovery path. We must not depend on it.

---

## 4.5 What step 1 instrumentation revealed (added 2026-05-24)

Step 1 (commit `04a736bd`) shipped the recording layer using a single process-wide `LAST_FOCUSED_CHILD: AtomicUsize`. Within minutes of running `task dev`, the host log showed **two distinct main render-widget HWNDs** being written to the same slot in quick succession:

```
[focus-track] LAST_FOCUSED_CHILD <= main hwnd=0x180f2a     # label "main"
[focus-track] LAST_FOCUSED_CHILD <= main hwnd=0x13710c4    # label "window-beb57746..."
```

AgentMux runs **multiple top-level windows in one process** — the primary `"main"` plus pool / sub-windows (see `agentmux-cef/src/state.rs` `list_browsers`, which exposes `"window-..."` labels alongside `"main"`). Each top-level has its own HWND, its own CEF browser, and its own keyboard-focus child. A single global slot overwrites whichever was written last, so a `WM_ACTIVATE` on window A would read window B's child and `SetFocus` the wrong one — exactly the inverse of the fix.

The storage layer must therefore be **per-top-level-window**, keyed by the root HWND. §5.1 below reflects this; the test matrix in §7 grew multi-window cases (L1.4-5, L3.5).

The empirical lesson: a one-line atomic looked sufficient on paper because the spec was written from the single-window mental model. Step 1's instrumentation surfaced the second window before any behavioural code shipped — exactly what the "instrumentation first" delivery order is for.

---

## 5. The fix

Two coordinated fixes, in order of robustness:

### 5.1 Rust host: top-level `WM_ACTIVATE` → delegate focus to the last-active child of *that window* (primary)

Track the last-focused CEF child HWND **per top-level window**. On `WM_ACTIVATE` of a given top-level HWND, look up the child for *that* window only, then call `SetFocus` on it and `browser_host->SetFocus(true)` on the matching CEF browser.

#### 5.1.1 Storage — per-root map

Replace the single `LAST_FOCUSED_CHILD: AtomicUsize` from step 1 (commit `04a736bd`) with a map keyed by root HWND:

```rust
// agentmux-cef/src/browser_pane/hwnd.rs
//
// Per-top-level-window record of the last child HWND to receive
// intentional keyboard focus. Keyed by GetAncestor(child, GA_ROOT).
// Mutex (not RwLock): writes are rare (per intentional focus event),
// reads rarer still (per WM_ACTIVATE), contention is negligible.
pub static LAST_FOCUSED_BY_ROOT: LazyLock<Mutex<HashMap<usize, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
```

Motivation: §4.5.

#### 5.1.2 Write path — single helper, both call sites

Both the pane subclass (`hwnd.rs` `wndproc_hook`, WM_SETFOCUS allowed-through branch) and the main reclaim (`ui_tasks.rs` `MainFocusReclaimTask`) call one helper:

```rust
unsafe fn record_intentional_focus(child: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
    let root = GetAncestor(child, GA_ROOT);
    if root.is_null() { return; }
    if let Ok(mut map) = LAST_FOCUSED_BY_ROOT.lock() {
        map.insert(root as usize, child as usize);
        tracing::info!(
            "[focus-track] LAST_FOCUSED_BY_ROOT[root={:p}] <= child={:p}",
            root, child,
        );
    }
}
```

`WM_KILLFOCUS` is still a no-op — entries persist after focus leaves the child so the next `WM_ACTIVATE` on that root has a target to restore.

#### 5.1.3 Read path — top-level `WM_ACTIVATE` handler

Subclass each top-level host HWND in the module that owns its `WndProc` (`agentmux-cef/src/window/...`). Handle `WM_ACTIVATE`:

```rust
const WA_INACTIVE: u32 = 0;

if msg == WM_ACTIVATE {
    let activation_state = (wparam & 0xFFFF) as u32;
    if activation_state != WA_INACTIVE {
        // `hwnd` here IS the top-level being activated, so it's its own root.
        let child = LAST_FOCUSED_BY_ROOT
            .lock()
            .ok()
            .and_then(|m| m.get(&(hwnd as usize)).copied())
            .unwrap_or(0);
        if child != 0 {
            let child_hwnd = child as *mut std::ffi::c_void;
            if IsWindow(child_hwnd) != 0 {
                ALLOW_BROWSER_PANE_FOCUS_ONCE.store(true, Ordering::Relaxed);
                SetFocus(child_hwnd);
                // If `child` belongs to a CEF browser, sync Chromium's
                // internal focus tracker too. Look up via state.list_browsers,
                // mirroring MainFocusReclaimTask's pattern.
                if let Some(browser) = find_browser_owning_child(&state, child_hwnd) {
                    if let Some(host) = browser.host() {
                        host.set_focus(1);
                    }
                }
                tracing::info!(
                    "[focus-restore] WM_ACTIVATE on root={:p} -> SetFocus child={:p}",
                    hwnd, child_hwnd,
                );
            }
        }
    }
}
```

A `PostMessage` follow-up notifies the frontend so it can complete the DOM half (§5.2). The post is *fire and forget*; if the frontend never sees it (e.g. main browser still loading), §5.2's `window`-focus listener fires anyway via Chromium's own focus event.

#### 5.1.4 Cleanup

Stale entries (child HWND destroyed) are self-healing: the `IsWindow` guard on activate turns a dead child into a no-op. Map growth is bounded by the number of top-level windows the process has ever owned — small. Optional follow-up: drop the entry on the top-level's `WM_NCDESTROY`. Not required for correctness.

#### 5.1.5 Why this is the primary fix

It restores focus at the Win32 + CEF layers, which is where the chain is actually broken. The frontend half (§5.2) cannot fix a missing `SetFocus(child_hwnd)` — it can only `.focus()` a DOM element *after* Chromium believes its browser has keyboard focus.

### 5.2 Frontend: `window` focus listener → `giveFocus()` on active pane (belt-and-braces)

Add to `frontend/app/app.tsx` (replacing the disabled `AppFocusHandler` stub at `:195-230`):

```ts
onMount(() => {
    const onWindowFocus = () => {
        // Read the currently-focused pane from the global focus tracker.
        const active = FocusManager.activeBlockNodeModel();
        if (!active) return;
        // Resolve to its view model and call giveFocus() if defined.
        const vm = viewModelForBlock(active.blockId);
        vm?.giveFocus?.();
    };
    window.addEventListener("focus", onWindowFocus);
    onCleanup(() => window.removeEventListener("focus", onWindowFocus));
});
```

The handler is idempotent and cheap; it runs whenever the window's DOM `focus` event fires (which is when Chromium believes the renderer has keyboard focus). After §5.1, this event fires reliably on re-activation; §5.2 then ensures DOM-level focus lands on the right input element rather than `body`.

### 5.3 Why both halves are needed

| Scenario | §5.1 alone | §5.2 alone | Both |
|---|---|---|---|
| `WM_ACTIVATE` fires, child gets `SetFocus`, terminal already had DOM focus before switch-away | ✅ | ❌ (Chromium never gets keyboard focus, `window` focus event does not fire) | ✅ |
| `WM_ACTIVATE` fires, child gets `SetFocus`, DOM focus had drifted to `body` | ⚠️ (Chromium focused but body, not xterm, is `activeElement`) | ❌ | ✅ |
| Active pane is the **main** browser (not a pane), focus had drifted | ✅ | ❌ | ✅ |

The two fixes target two distinct levels. Neither alone is sufficient.

---

## 6. Out of scope

- **Linux and macOS.** The bug is reported on Windows, and the host architecture there uses CEF child windows that paint above the DOM. Other platforms have different focus semantics; fixing them is a separate effort. (`task dev` on Linux/macOS still invokes the host directly — see `CLAUDE.md`.)
- **Browser pane input.** A focused browser pane manages its own input via Chromium; no change to that path.
- **First-launch focus.** Focus on splash → first window is a different code path and is not affected by this spec.
- **Multi-monitor / DPI edge cases.** Same handler, no DPI-specific logic required.
- **Refactoring `MainFocusReclaimTask`.** The existing pane→main reclaim path keeps working unchanged; §5.1 adds a *new* path for window-reactivate, it does not modify the existing path.
- **Replacing `ALLOW_BROWSER_PANE_FOCUS_ONCE` with a proper state model.** The one-shot flag is fine for both the existing IPC path and the new activate path. Re-architecting it can wait for `SPEC_BROWSER_PANE_FOCUS_LOCK.md` if/when that lands.

---

## 7. Tests

### L1 — Rust unit (`agentmux-cef/src/window/...`)

- **L1.1** `record_intentional_focus(child)` inserts `(GetAncestor(child, GA_ROOT), child)` into `LAST_FOCUSED_BY_ROOT`. Mock `GetAncestor`, assert the entry.
- **L1.2** `WM_ACTIVATE(WA_ACTIVE)` on a root with a valid map entry calls `SetFocus` on that child (mock `SetFocus`, assert the argument is the child not the root).
- **L1.3** `WM_ACTIVATE(WA_ACTIVE)` on a root with no entry is a no-op.
- **L1.4** Two roots, two entries: `WM_ACTIVATE` on root A reads child A; on root B reads child B. Critical regression guard against the §4.5 finding — a single-slot impl fails this.
- **L1.5** Entry whose child fails `IsWindow` causes activate to no-op (mock `IsWindow` to return 0). Stale entries are tolerated.
- **L1.6** `WM_ACTIVATE(WA_INACTIVE)` does not mutate the map.

### L2 — Frontend unit

- The `window` focus handler calls `giveFocus()` on the active block's view model.
- The handler is a no-op when no block is active.
- The handler is removed on `onCleanup`.

### L3 — Integration / manual

- Open AgentMux, open a terminal pane, focus it, type — works.
- Alt+Tab to another app, Alt+Tab back, type — works (today: intermittently fails).
- Repeat 20× with random delays — should be 20/20.
- Click taskbar to switch away, click taskbar to return, type — works.
- **L3.5 (multi-window, critical)** Open the primary window + a sub/pool window. Focus a terminal in window A; focus a terminal in window B. Alt+Tab away, Alt+Tab back to A → typing reaches A's terminal, NOT B's. Repeat in the other direction. Cross-window focus must never leak — direct test of the §4.5 finding.
- Active pane is an **agent** pane (input is a `contenteditable`, not xterm) — focus lands on it.
- Active pane is an **editor** pane — focus lands on the editor.

---

## 8. Order of delivery

One commit per step, on a feature branch (`agentx/window-reactivate-focus-restore`):

1. **Shipped (`04a736bd`).** Add `LAST_FOCUSED_CHILD: AtomicUsize` recording to the existing browser-pane subclass and to `MainFocusReclaimTask`. No behaviour change — pure instrumentation. Step 1 surfaced the multi-window evidence in §4.5.
2. **Step 1.5 (new — added after the §4.5 finding).** Refactor storage to per-root `LAST_FOCUSED_BY_ROOT: HashMap<root_hwnd, child_hwnd>` (§5.1.1). Both write sites route through the `record_intentional_focus` helper (§5.1.2). Still no behavioural change; logs gain a `root=` field. Lands L1.1.
3. Install the top-level `WM_ACTIVATE` handler (§5.1.3), reading from the per-root map. `SetFocus(child)` + `browser_host.set_focus(1)`. Resolves most occurrences. Lands L1.2-L1.6.
4. Replace the disabled `AppFocusHandler` stub at `frontend/app/app.tsx:195-230` with an active `window` focus listener that calls `giveFocus()` on the active pane (§5.2). Lands L2 tests.
5. Manual L3 verification on the dev build, including the L3.5 multi-window case. Ship behind no flag — this is pure recovery code, additive, with no negative path.

Each commit is independently revertible; revert order is 5 → 1 if needed. Step 2 (1.5) MUST land before step 3 — the activate handler depends on the per-root map.

---

## 9. Related

- `docs/specs/SPEC_BROWSER_PANE_FOCUS_LOCK.md` (draft) — covers intra-app Chromium/Win32 focus desync. Adjacent, not overlapping; this spec depends only on the existing `ALLOW_BROWSER_PANE_FOCUS_ONCE` flag it introduced.
- `docs/specs/SPEC_PANE_FOCUS_STRESS_TEST.md` — pre-existing focus reliability test surface; the §7 L3 cases should be folded in.
- `agentmux-cef/src/ui_tasks.rs` `MainFocusReclaimTask` — the analogous reclaim path for the *inverse* direction (pane → main). The §5.1 activate handler mirrors its structure.
