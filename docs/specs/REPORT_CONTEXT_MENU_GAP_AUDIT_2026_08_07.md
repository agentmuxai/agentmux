# REPORT — context-menu gap audit: Swarm copy, agent-pane paste, and beyond

**Date:** 2026-08-07
**Trigger:** Direct ask — add a Copy menu on Swarm right-click and a Paste
menu on the agent-pane composer's right-click, then scour the app for the
same class of gap.
**Scope:** Diagnosis + fix for the two requested spots, plus every other
free-text `<input>`/`<textarea>` found to have the identical bug. List/table
Copy-menu polish beyond Swarm (drone/warden/list managers) and low-value
single-line inputs (settings, file-tree rename, editor "Save As", toolchain,
identity account form) are catalogued but intentionally left for later.

---

## 1. Root cause (one bug, ~20 symptoms)

Two independent right-click handlers exist, and whichever is closer to the
clicked element in the DOM wins (event bubbles from the target outward):

1. **`block/blockframe.tsx`'s `onBodyContextMenu`** — attached to every
   pane's body `<div>`. Calls `preventDefault`/`stopPropagation` and shows
   `buildPaneContextMenu()` (`block/pane-actions.ts`): Copy only if there's
   an active `window.getSelection()`, Paste **only** when
   `blockData.meta?.view === "term"`, plus Split/Replace/Close.
2. **`app.tsx`'s root-level `handleContextMenu`** — a document-root
   fallback that shows real Cut/Copy/Paste based on
   `document.activeElement`'s tag. Portalled `Modal`/`ModalLayer` content
   renders outside the pane-body DOM subtree, so its context-menu events
   never pass through (1) and reach this fallback untouched — modal inputs
   already worked correctly before this fix.

Since (1) sits between any in-pane element and (2), and non-`term` panes
never offer Paste, **every `<input>`/`<textarea>` living directly in a
non-terminal pane's body — not inside a portalled modal — had no way to
paste via right-click**: the click gets hijacked into a useless
disabled-Copy menu instead. `agent-view.tsx`'s existing `handleContextMenu`
(a Copy-on-selection handler covering the transcript) made this worse for
the composer specifically: it doesn't call `preventDefault`/`stopPropagation`
when there's no selection, so an empty/partially-typed composer's
right-click falls through to (1) anyway.

Confirmed via full-file greps: `frontend/app/view/swarm/` has zero
context-menu wiring anywhere, and its rows are `user-select: none`
(`swarm-view.scss`), so right-click there did literally nothing — not even
degraded to the generic Copy-on-selection fallback.

## 2. The fix

**Shared helper, single source of truth.** `app.tsx`'s root fallback logic
(`canEnableCut/Copy/Paste`, clipboard-URL detection, the Cut/Copy/Paste role
menu) moved into `frontend/app/store/contextmenu.ts` as an exported
`showTextInputContextMenu(e: MouseEvent)`, colocated with `ContextMenuModel`
it already depends on. `app.tsx` now imports and uses it instead of a local
duplicate — no behavior change for the root-fallback path itself.

**Per-element opt-in.** Every affected `<input>`/`<textarea>` gets
`onContextMenu={showTextInputContextMenu}` directly. Because the handler
calls `stopPropagation`, attaching it on the element always wins over
`blockframe.tsx`'s pane-body handler for that element specifically — the
same mechanism that already made modal inputs work, just applied explicitly
instead of relying on a portal to route around the problem.

**Swarm Copy menu.** The primary agent row (`AgentRow`'s `.swarm-agent-card`
in `swarm-view.tsx`) gets a native `ContextMenuModel` menu ("Copy agent
name", "Copy block ID") — the `AgentPicker.tsx` per-row pattern
(`handleTemplateContextMenu`) was the template. Scoped to the primary row
only; shell/cron/subagent/workflow sub-rows are cataloged in §4 as
follow-up, not fixed here.

## 3. Files changed

| File | Fix |
|---|---|
| `app/store/contextmenu.ts` | New exported `showTextInputContextMenu`; moved `canEnableCut/Copy/Paste`, `getClipboardURL` here from `app.tsx` |
| `app/app.tsx` | Uses the shared helper instead of a local duplicate |
| `app/view/agent/components/AgentFooter.tsx` | Composer textarea (the original ask) |
| `app/view/swarm/swarm-view.tsx` | Copy menu on the primary agent row (the original ask) |
| `app/view/agent/components/AgentDecisionPanel.tsx` | Deny-feedback textarea |
| `app/view/agent/components/AgentQuestionPanel.tsx` | "Other" free-text answer input |
| `app/view/browser/browser-nav-bar.tsx` | Address bar |
| `app/view/skill/skill-manager.tsx` | Name/trigger/description/content fields |
| `app/view/mcp/mcp-manager.tsx` | Name/transport/config fields |
| `app/view/memory/memory-manager.tsx` | Name/description/instructions fields |
| `app/view/brain/global-brain-manager.tsx` | Name/content fields |
| `app/view/drone/drone-view.tsx` | Drone name + all node-editor fields (task/URL/body/condition/template/variables/instance) |

## 4. Catalogued, not yet fixed

Found during the sweep, deliberately out of scope for this pass:

- **List/table Copy-id polish** — `warden.tsx` (host/agent table + audit
  feed), and the skill/MCP/memory/brain-manager list rows: generic
  selection-Copy technically works (not `user-select: none`), but there's
  no contextual "Copy id" the way the new Swarm menu has. `drone-view.tsx`
  node/workflow labels ARE `user-select: none` (same total-gap pattern as
  Swarm) and would need the same treatment Swarm just got.
- **Low-value single-line inputs**, same underlying bug but short/rarely-pasted
  values: `view/toolchain/toolchain-view.tsx`, `view/editor/editor-tab-strip.tsx`
  ("Save As" path), `view/editor/file-tree.tsx` (rename/new-file),
  `view/settings/**`, `view/identity/identity-account-form.tsx` (needs a
  quick check whether it's ever rendered inline vs. only inside a modal
  before fixing — not yet confirmed either way).
- `AgentSearchBar.tsx`'s in-transcript search input and
  `MyAgentsList.tsx`'s fork "Session name" input — same bug, lower value
  since both are typically typed fresh rather than pasted into.
