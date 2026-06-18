# Analysis: Floating-pane tear-off — what's in, what's needed

**Date:** 2026-05-26
**Status:** Investigation (no code change yet)
**Companion to:** `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md`

## TL;DR

A 6-phase spec already exists for exactly the feature requested: tear a
pane out and have it become an owned `WS_POPUP | WS_EX_TOOLWINDOW`
floating child of the parent window — pane content only, no taskbar
entry, no Alt-Tab, minimizes/restores/destroys with the parent, shares
the same sidecar.

**Phase 1 of that spec is already merged** (PR #810, commit
`944aeec2`, 2026-05-11). The Windows host primitive that creates the
owned no-taskbar HWND + embeds CEF + redirects focus is in place. What
ships today is a placeholder shell card; the surrounding wiring (drag
gesture, routing, reducer state) is not.

The behavior the user dislikes — pane tear-off creating a full new
top-level instance with its own taskbar entry, tabs, widgets, and
status bar — comes from the tab tear-off code path being reused for
pane drags. Both currently terminate in
`agentmux-cef/src/commands/drag.rs::open_window_at_position`, which
spawns a brand-new CEF top-level window.

The MVP is three small follow-ups on top of Phase 1, all spec'd.

---

## 1. Existing specs (in this area)

| File | Date | Status | Summary |
|---|---|---|---|
| `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` | 2026-05-11 | Proposed; Phase 1 merged | **Directly addresses this feature.** Owned `WS_POPUP \| WS_EX_TOOLWINDOW` child window, no taskbar, shared sidecar. 6-phase rollout. |
| `docs/specs/PLAN_TAB_TEAROFF_PHASE1_WIN32_2026-05-07.md` | 2026-05-07 | Ready to implement | Native Win32 drag loop with pointer capture for **tab** tear-off. Replaces pragmatic-dnd's HTML5 drag. |
| `docs/specs/RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md` | 2026-05-07 | Decision | Cross-platform drag-preview research. Recommends bitmap snapshot (static, all platforms) over per-platform custom drag loops. |
| `docs/specs/tearoff-pane-size.md` | 2026-04-07 | Draft | Capture source pane's rect + grab offset on drag start, position torn window so the cursor lands at the same relative point. |
| `docs/specs/cef-drag-window-management.md` | 2026-03-29 | Analysis + phased plan | Earliest cross-window-drag analysis; Phase 3 (multi-window + cross-window drag) is the umbrella that the per-pane and per-tab specs nest under. |

## 2. Currently shipped tear-off (tabs)

The tab tear-off the user has now:

- **Gesture detect** (frontend) — drag a tab below the tab bar by
  ≥5 px or release-outside-window.
  - `frontend/app/tab/tabbar.tsx` — `requestTearOff` around line 232.
  - `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` — `performTearOff`
    around line 123. Listens for `dragend` outside-the-window and routes.
- **Drag payload** — `setCurrentDragPayload({ kind: "tab" | "tile", … })`
  established on drag start. The pane case uses `kind: "tile"`.
- **Backend handoff** — `WorkspaceService.TearOffBlock` moves the
  block to a new workspace in the reducer.
- **Host spawn** — `agentmux-cef/src/commands/drag.rs::open_window_at_position`
  (around line 26). Creates a **new top-level CEF window** at the
  cursor's screen position. Hardcoded 1200×800 — being fixed by the
  `tearoff-pane-size.md` spec to capture the source rect.
- **Outcome** — new full AgentMux instance. Own sidecar, own taskbar
  entry, own data dir. The user's described "tabs/widgets/status bar
  show up too" comes from this being a full window, not a popup.

Pane drag rides the same drop path. When a tile is dragged out of the
layout and released outside the window, `performTearOff` is called
with `dragType === "tile"` and routes to the same
`open_window_at_position` call. Hence: full new instance.

## 3. Phase 1 primitive — already merged

`agentmux-cef/src/floating_pane.rs` + `agentmux-cef/src/commands/floating_pane.rs`
(landed in PR #810, May 11):

- Finds the calling instance's main HWND via
  `find_own_top_level_window()` so we know whose child to be.
- Creates a new HWND with `WS_POPUP | WS_EX_TOOLWINDOW` and the main
  HWND as **owner** — that gives us:
  - No taskbar entry.
  - No Alt-Tab entry.
  - Auto-minimize / restore / destroy with the parent.
- Embeds a CEF browser via `WindowInfo::set_as_child(outerHwnd, CefRect{…})`
  — same pattern as the existing browser-pane embedding.
- Focus chain reuses the existing `install_browser_pane_focus_redirect`
  subclass hook (`agentmux-cef/src/browser_pane/hwnd.rs:~20-100`) so
  `WM_SETFOCUS` redirects to the parent unless a one-shot allow flag is
  set — same plumbing the regular panes already use.

The IPC entry point is `open_floating_pane_window(paneId, x, y, w, h)`
in `frontend/util/cef-api.ts`.

The frontend shell mounted inside that browser is currently a
placeholder card at `frontend/app/floating-pane/floating-pane-shell.tsx`
showing `Floating pane: <id>` — that's Phase 1's stub.

## 4. Pane-level drag UI today

- `frontend/layout/lib/TileLayout.win32.tsx:~461` — the whole tile is
  draggable via pragmatic-dnd (`<Draggable>`). No explicit drag handle
  on pane chrome.
- `setCurrentDragPayload({ kind: "tile", node })` at drag start.
- `CrossWindowDragMonitor.win32.tsx` (lines ~42-211) watches for
  drag-end outside the window and calls `performTearOff`.
- No `ResizeObserver` is wired to pane-tear geometry; only used by
  `frontend/app/block/pane-size-badge.tsx` for the WxH badge during
  resize.

So there's already a working pane drag path; it's just terminating at
the wrong target.

## 5. Branches / PRs in flight from other agents

- `origin/agent1/floating-pane-phase1` — likely Phase 2 / 3 WIP from
  Agent1. **Worth inspecting before duplicating work.**
- `origin/agenta/feat-tearoff-match-source-size` — implementing
  `tearoff-pane-size.md` (capture source rect + grab offset).
- `origin/agenta/tear-off-phase2`, `phase4`, `phase6` — refinements on
  the **tab** tear-off code path.

No open issues in `agentmuxai/agentmux` literally titled "pane tear" or
"floating pane" — the work is tracked via specs + branches rather than
public issues.

## 6. CLAUDE.md context

`agentmux/CLAUDE.md` (`### Multiple Instances Run in Parallel`, line
~78-79) only describes tab tear-off, with the caveat that the result
is a full new instance. The CLAUDE.md doesn't mention floating panes —
it predates the spec.

---

## 7. Gap analysis

### Reusable (already in place)

1. **Drag detection threshold + IPC payload** (from tab tear-off).
   `setCurrentDragPayload` + `dragend` outside-window detection + the
   `screenX/screenY` cursor coords work for both kinds of drag.
2. **CEF host primitive for owned child HWND** (Phase 1 shipped):
   `open_floating_pane_window` creates the no-taskbar window, embeds
   the CEF browser, handles focus + DPI.
3. **Block renderer** (`frontend/app/block/block.tsx`) is mode-agnostic
   — same component works docked or floating.

### Missing for MVP (Phases 2 + 3)

1. **Floating-pane shell** (Phase 2) — `floating-pane-shell.tsx` is a
   placeholder. Replace with `<Block>` for the given `paneId`, plus
   the sidecar WebSocket subscription to `block:update`. No tab bar,
   widgets bar, or status bar — those belong to the main window only.
2. **Pane drag gesture** — currently the whole tile is draggable for
   intra-layout reorder. To tear a pane out as a floater rather than
   into a full new window, the gesture has to be **distinguishable**.
   Two options:
   - **Drag handle on pane chrome** (like browser DevTools): only the
     handle initiates tear-to-float. Whole-tile drag stays for
     in-layout reorder.
   - **Modifier-based** (e.g. Shift+drag): same gesture, different
     intent flag in the payload.
   The spec doesn't lock this in; needs a small UI decision.
3. **Tear-off routing branch** (Phase 3) — in
   `CrossWindowDragMonitor.win32.tsx::performTearOff`:
   ```
   if (dragType === "tile" && intent === "float")
       openFloatingPaneWindow(paneId, x, y, w, h)
   else
       openWindowAtPosition(x, y, newWsId)        // existing
   ```
4. **Reducer command `MarkPaneFloating`** (Phase 3) — moves the pane
   from the tile tree into a new `floating: { paneId → { monitor, x,
   y, w, h } }` map on the layout. The spec specifies the shape.

### Out of scope for MVP (Phases 4–6)

- Re-dock: dragging a floater's title bar back into the source
  window's layout / tab bar.
- Floater geometry persistence across restart.
- Cross-instance floaters (probably never — floaters are
  intentionally bound to the parent instance).

---

## 8. Resolved open questions

### 8.1 Agent1's branch is the merged Phase 1 — nothing else there

Inspected `origin/agent1/floating-pane-phase1`. It contains exactly:

- `69ddd134 feat(floating-pane): Phase 1 — host primitive + frontend stub (#810)` — already merged into main
- `933a2e3d chore: bump version to 0.33.807` — stale local bump
- `cfbccd7c chore: sync package-lock.json` — stale lockfile

No Phase 2 / 3 work. The branch isn't blocking; new work can land on
top of `origin/main` directly.

### 8.2 Gesture choice — recommend an explicit drag handle on pane chrome

Two options, tradeoffs:

| Option | Pros | Cons |
|---|---|---|
| **Drag handle on pane chrome** (recommended) | Discoverable; whole-tile drag stays as "reorder within layout"; matches DevTools / VS Code "move into new window" patterns | Tiny UI surface to add (one new control on each pane's title bar) |
| **Modifier+drag** (e.g. Shift+drag) | Zero UI surface | Undiscoverable; users won't find it without docs; relies on key-state tracking through the drag |

Pick the drag handle. It's the convention every comparable app (VS
Code, Photoshop, Figma, browser DevTools) uses, and the spec's
"dock-back" button on the floater title bar implies the same chrome
language anyway.

### 8.3 MVP scope — Phase 2 + Phase 3 from the spec, Windows first

Phase 2 ships the floating-pane shell with the real Block renderer
(replacing the placeholder card). Phase 3 routes the pane-drag gesture
to `openFloatingPaneWindow` and adds the `MarkPaneFloating` reducer
command. That gives the user the behavior they described.

Phases 4–6 (re-dock, geometry persistence, polish) are follow-ups.

### 8.4 Cross-platform — Phase 1 is Windows-only; macOS and Linux need their own native window recipes

The existing spec (§10) explicitly defers cross-platform. The
companion spec at
`docs/specs/SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md`
covers the macOS (`NSPanel.addChildWindow`) and Linux X11/GTK
(`set_transient_for` + utility type hint + skip-taskbar) recipes, with
a Wayland note (deferred — CEF Wayland support is still maturing and
agentmux uses CEF Ozone-X11 today).

## 9. References

- `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` — canonical
  spec; 6-phase plan.
- `agentmux-cef/src/floating_pane.rs` — Phase 1 host primitive.
- `agentmux-cef/src/commands/floating_pane.rs` — Phase 1 IPC entry.
- `frontend/app/floating-pane/floating-pane-shell.tsx` — Phase 1
  placeholder shell.
- `agentmux-cef/src/commands/drag.rs::open_window_at_position` —
  current new-top-level-window spawn used by tab tear-off + (today)
  pane tear-off.
- `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` —
  `performTearOff` routing logic; Phase 3 modifies this.
- `frontend/app/tab/tabbar.tsx` — tab tear-off gesture detection.
- `frontend/layout/lib/TileLayout.win32.tsx` — current pane drag (via
  pragmatic-dnd).
- `agentmux-cef/src/browser_pane/hwnd.rs` — focus-redirect subclass
  hook reused by Phase 1 floaters.
- PR #810 — Phase 1 merge (`944aeec2`, 2026-05-11).
