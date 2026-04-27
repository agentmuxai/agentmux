# Spec: Version-Click Instance Panel

**Date:** 2026-04-25 (revised 2026-04-26 to reflect arch drift)
**Status:** Draft, ready to implement (V1 — per-window tokens deferred)
**Owner:** AgentA

## Drift since 2026-04-25 (verified 2026-04-26)

Three items the original draft assumed don't match the current
codebase. Two are fortunate (less work), one is scope-trimming:

1. **`getApi().focusWindow(label)` already exists** at
   `agentmux-cef/src/commands/window.rs:324-348`, complete with
   `SetForegroundWindow` already imported. §5.1 is no-op.
2. **`token-usage` store is global / session-scoped, NOT
   per-window** (`frontend/app/store/token-usage.ts:14`). The V1
   ships **without** per-window token totals in the window list
   to avoid blocking on a token-usage refactor. The row format
   simplifies to `Window N · focused-agent · started-at`. A
   follow-up can extend token-usage per-window and add the totals.
3. ~~**`window-instances-changed` event does not exist**~~ —
   correction on second look: it DOES exist. Emitted at
   `agentmux-cef/src/commands/window.rs:514` on window create with
   the new count as payload. (May need a second emit on destroy
   if not already present — verify before adding.)

**V1 scope adjustment** — LAN peers section dropped from this
panel. `HostPopover` already shows LAN with hover-rich detail;
duplicating it here would be confusing UX. The version panel V1
is: **about-info + this-process windows + actions**. LAN stays in
HostPopover.

**V1 scope adjustment** — per-window data simplified to
`{ label, instanceNum, isCurrent }` (drop `focusedAgent*`,
`startedAt`, `tokens` from the row). `getApi().listWindows()`
already returns the labels we need; richer per-window data is a
follow-up that requires extending the host registry.

Bonus: `AboutModalDetails` TS type at `frontend/types/custom.d.ts`
has `buildTime: number` but the Rust handler at
`agentmux-cef/src/commands/platform.rs:127-140` returns string,
plus `platform`, `arch`, `backendEndpoints` fields the TS type
omits. V1 fixes the type.
**Touches:** `frontend/app/statusbar/StatusBar.tsx`,
             new `frontend/app/statusbar/InstancePanel.tsx`,
             new `frontend/app/statusbar/_instance-panel.scss`,
             `frontend/app/store/global.ts` (read-only —
             consumes existing window/LAN atoms)

---

## 1. Problem

Clicking `vX.X.X` in the bottom-right of the status bar today
opens a new AgentMux window via `getApi().openNewWindow()`.
That's a useful action but it's the *only* thing the user can
do, and it gives up the version chip's natural role as an
"about / status" affordance:

- No way to see what other windows are open in this AgentMux
  process, or what's running in them.
- LAN peer instances exist (`lanInstancesAtom`) but only
  surface in the separate `HostPopover` widget that most
  users don't notice.
- There's no consolidated "About this app" panel — the
  version number lives next to a click handler that has
  nothing to do with the version itself.

## 2. Goals

- **G1.** Clicking the version chip opens a popover anchored
  to it.
- **G2.** The popover shows the open windows in this
  AgentMux process: window index, focused agent (if any),
  started-at, link to focus.
- **G3.** Surfaces LAN-discovered peer instances (consume
  `lanInstancesAtom` directly — no duplicate plumbing).
- **G4.** "Open another window" button stays prominent
  (preserves today's primary action).
- **G5.** Includes "About" metadata: full version, commit
  short-sha when known, build date, runtime (Windows /
  macOS / Linux), and a copy-to-clipboard affordance for
  bug-report convenience.
- **G6.** Reuses the airspace primitive (`usePaneOverlay`)
  so the popover paints over any browser pane HWND — same
  as MoreDropdown, modal-v2, TokenBreakdownPopover.
- **G7.** Edge-anchored so it doesn't overflow the right
  edge of the window. Same pattern as the status-bar
  tooltips from PR #553.

## 3. Non-goals

- **No remote control** of LAN peers from this panel. They
  are read-only listings; existing host-popover logic stays.
- **No per-window kill / close**. Clicking a window switches
  focus to it; that's all.
- **No history of *closed* windows.** Snapshot of the live
  process state.
- **No theme picker, settings shortcut, or telemetry
  toggle.** Those belong in settings — out of scope.

---

## 4. Design

### 4.1 Anchor + open

The version chip becomes a button (`<button>` not `<span>`)
with `aria-haspopup="dialog"` and `aria-expanded`. Click
toggles the popover open/closed. Esc closes when open.
Click-outside closes (same handler pattern as
TokenBreakdownPopover).

The previous primary action — "open new window" — is now
the panel's confirm button instead of a side-effect of the
chip click. New users discover it; existing users get one
extra click but a much richer affordance.

### 4.2 Layout

```
┌ AgentMux ──────────────────────────────────────┐
│ Version  v0.33.392  [📋 copy]                  │
│ Build    2026-04-25 · windows-x86_64           │
│ Commit   bd7d90a (click to copy)               │
├────────────────────────────────────────────────┤
│ This process — 2 windows                        │
│ ● Window 1 (focused)    Claude — feat/auth     │
│   started 14:02 · 18 turns · ↑42k ↓12k         │
│ ○ Window 2              Codex — bugfix/queue   │
│   started 14:31 · 4 turns · ↑8k ↓2k            │
├────────────────────────────────────────────────┤
│ LAN peers — 1 visible                           │
│ ◇ kona.local (asaf)    v0.33.388 · 5 panes     │
├────────────────────────────────────────────────┤
│ [+ Open another window]              [Close]   │
└────────────────────────────────────────────────┘
```

### 4.3 Sections, top to bottom

**Header — App identity (G5).**
- Full version (`vX.Y.Z`).
- Build date + platform string.
- Commit short-sha (when present in the build artifact).
- Each row has a small copy icon that copies the value to
  the clipboard for bug-report convenience.

**This process — open windows (G2).**
- Header row: `"This process — N windows"` where N is
  `windowCountAtom`.
- One row per window. Today's data we already have:
  - Window instance number (`windowInstanceNumAtom` is
    *this* window — others come from a sidecar registry).
  - Focused / not-focused indicator (●/○).
- New data we should expose so the rows are useful:
  - Currently-focused agent slug + name (if any).
  - Started-at timestamp.
  - Per-window token totals (from the
    `token-usage` store, scoped per window).
- Click a row → focus that window via a new
  `getApi().focusWindow(label)` call. Behaviour parity
  with how the OS taskbar would handle a click.

**LAN peers — discovered instances (G3).**
- Reads `lanInstancesAtom` directly.
- Displays the same compact rows that today's
  `HostPopover` shows. If the LAN list is empty, hide the
  whole section (don't render an empty "no peers" line —
  most users never have peers; an empty section adds
  visual debt).
- Read-only: no click action on peers (G3 / non-goal).

**Footer — actions.**
- Primary button: **`+ Open another window`** — calls the
  existing `getApi().openNewWindow()`. Same effect as
  today's chip click, just one click later.
- Secondary button: **Close** — dismisses the panel
  without action. Esc and click-outside do the same.

### 4.4 Empty / degraded states

- **Single-window process (the common case):** the "This
  process" section shows just one row. Still rendered, so
  users learn the section exists for when they do open a
  second.
- **Backend offline (`backendStatusAtom === "crashed"`):**
  some live data (per-window token totals, focus indicator)
  may be stale. Render last-known values with a small
  "(backend offline)" caption above the section. Don't
  block the popover from opening.
- **Build metadata missing:** if the host couldn't supply
  commit / build-date, render `"unknown"` rather than
  hiding the row — empty rows look like a UI bug.

## 5. Plumbing required

### 5.1 New host API

**(Already exists — no work)** `getApi().focusWindow(label)` is
implemented at `agentmux-cef/src/commands/window.rs:324-348` with
`SetForegroundWindow` already imported. Wire the panel's
"click row to focus" handler to it directly.

### 5.2 New frontend atom (V1 — token totals deferred)

`openWindowsAtom: Accessor<WindowInstance[]>` in
`frontend/app/store/global.ts`. Each `WindowInstance`:

```ts
interface WindowInstance {
    label: string;             // "main", "window-2", …
    instanceNum: number;
    focusedAgentId?: string;   // current pane's agent slug, if any
    focusedAgentName?: string;
    startedAt: number;         // epoch ms
    isFocused: boolean;        // distinct from "this window"
    // tokens: deferred — needs token-usage per-window refactor.
}
```

Source of truth: a new host event `window-instances-changed`
(emitted from the create / destroy paths in
`agentmux-cef/src/commands/window.rs`). Frontend listens via
the existing event-bus pattern, builds the array.

### 5.3 New build-info API

`getApi().getBuildInfo(): Promise<{ version: string;
commit?: string; buildDate?: string; platform: string }>`.
Reads from the existing about-modal-details surface so
this is mostly a re-shape, not new plumbing.

## 6. Interaction model

**Click version chip:** open / close popover (toggle).
**Esc / click-outside:** close (per
TokenBreakdownPopover precedent).
**Click a window row:** focus that window. Popover stays
open so the user can see the change reflected (focus
indicator updates).
**Click "Open another window":** call
`getApi().openNewWindow()`. Popover closes.
**Click copy icon next to version / commit / build:**
write the value to clipboard, briefly flash the icon as
"copied" (reuse the existing `CopyButton` element).

## 7. Risks

| Risk | Mitigation |
|---|---|
| Per-window data dependency on a new host event | If the event isn't shipping yet, fall back to showing only the local window's data and a "(other windows offline)" caption. Not a blocker. |
| Popover overflow off the right edge of the window | Anchor with `right: 0; left: auto` — same pattern PR #553's status-bar tooltips use. |
| Clicking a *focused* window does nothing visibly — looks broken | Disable that row's click handler; render the focus indicator more prominently so it's clear it's already focused. |
| Cross-window state desync (per-instance DOM IDs) | Apply the rule from `SPEC_DECISION_PROMPT_DESIGN_2026_04_25 §8`: any DOM `id`/radio `name` is per-instance via `createUniqueId()`. |
| Single-window users see an "extra" button click for what used to be one click | Match the chip itself as the explicit "open" target via Shift+Click for power users (same fast-path), with the panel as the new default. |

## 8. Validation

- ✅ `task build:frontend` succeeds
- ✅ `tsc --noEmit` clean
- ✅ Stylelint green
- ✅ Manual smoke (`task dev`):
  - Single window → popover lists 1 window, "Open another
    window" works
  - After opening, popover shows 2 windows
  - Click the non-focused window → focus moves there
  - Esc / click-outside closes
  - With a browser pane visible behind the popover →
    popover paints cleanly (airspace works)
- ✅ Bug-report copy: clicking copy on each header row
  writes the right value to clipboard

## 9. Cross-references

- `frontend/app/statusbar/StatusBar.tsx` — version-chip
  current click handler
- `frontend/app/statusbar/HostPopover.tsx` — reference
  popover anchored from the status bar; LAN list source
- `frontend/app/statusbar/TokenBreakdownPopover.tsx` — the
  closest sibling; airspace + outside-click handling to
  copy
- `frontend/app/store/token-usage.ts` — per-window token
  scoping (extend if needed)
- `agentmux-cef/src/commands/window.rs` — where
  `focusWindow` would land
- `SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.3-4.4` —
  always-visible widget + edge-anchor patterns this panel
  inherits
- `SPEC_DECISION_PROMPT_DESIGN_2026_04_25 §8` —
  per-instance ID rule that applies here too
