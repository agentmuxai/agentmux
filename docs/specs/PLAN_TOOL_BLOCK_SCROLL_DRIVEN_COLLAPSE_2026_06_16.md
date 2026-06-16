# Implementation Plan: Scroll-Driven Tool-Block Collapse

**Date:** 2026-06-16
**Author:** smike (agent)
**Status:** In progress
**Companion analysis:** `docs/analysis/ANALYSIS_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md`

## Goal

A completed tool block stays **expanded while it's on screen**, and **collapses
once it scrolls off the top** (latched). The 3 s post-completion timer
(`ToolBlock.postCompletionHold`) is removed. Everything else — pin override,
hover-hold, auto-expand while running/pending, immediate collapse for
denied/canceled, the ✗/red collapsed-row treatment for failures — is unchanged.

## Model

A single pane-level set, `documentState.expandedTools: Set<string>` = "completed
tools currently held open." It is the post-completion hold, made durable across
unmount and shared by both the visual render and the virtualization layout.

- **ADD** `id` when a tool transitions `active → inactive` (completes **live, on
  screen**). This reuses `ToolBlock`'s existing transition effect (the same
  trigger that armed the 3 s timer), so loaded-history tools — which never
  transition this session — are never added and stay collapsed (no load-flash;
  preserves the guard at ToolBlock.tsx:86-89).
- **REMOVE** `id` when the tool's row has scrolled fully above the viewport top.
  This is the latch: once removed it won't be re-added (no new run), so scrolling
  back up leaves it collapsed (click-to-pin to re-open — "same as now").

`currentExpansion` (the single layout decider, expansion-source.ts) gains:
`if (state.expandedTools.has(id)) → { open: true, via: "auto" }`, so virtualized
row **heights match the visual** — closing the existing component-vs-layout
divergence the file's header documents (expansion-source.ts:24-31).

`ToolBlock.expanded()` becomes `pinned || running || pending || heldOpen ||
userHolding`, where `heldOpen = documentState.expandedTools.has(id)` (passed in
like `pinned`).

## Collapse trigger — DOM-based scan, stick-to-bottom-gated (Phase 1)

In `AgentDocumentVirtualList.handleScroll`, after the `Scrolled` dispatch, when
`stickToBottom()` is engaged, scan `documentState().expandedTools`: for each
unpinned held tool, look up its rendered row by `data-node-id` and release it if
its `getBoundingClientRect().bottom <= scrollRef` top (fully above the fold), or
if it has no element (already unmounted off-screen — safety net). DOM-based so it
works identically for virtualized and streaming-buffer rows, and is zoom-safe
(both rects are in the same zoomed space). `expandedTools` is tiny (a few recent
tools), so this is a handful of `querySelector`s per scroll.

**Why stick-to-bottom-gated:** collapsing a row above the fold shrinks layout
height above `scrollTop`. While pinned to bottom this is invisible (scrollHeight
shrinks, auto-scroll stays pinned — no jump). This covers the primary scenario:
live streaming pushes a just-completed tool up off the top → it collapses. When
the user is scrolled up reading history, held tools above the top are simply not
released until they return to the bottom (stick re-engages, scrolls to bottom,
collapse is invisible) — no jump in any case.

**Phase 2 (follow-up, separate commit):** relax the gate so a tool collapses when
scrolled-down past while *not* stuck to bottom, using the existing
`captureTopmostAnchor`/`restoreScrollFromAnchor` primitive to compensate
`scrollTop` by the height delta. Lower priority (those tools are off-screen; only
the scroll position, not the collapse, is visible).

## Edits

1. **`types.ts`** — add `expandedTools: Set<string>` to `DocumentState`.
2. **Construction sites** — init `expandedTools: new Set()`:
   `state.ts:137`, `useHistoryPagination.ts:276` (fresh, not persisted),
   `expansion-source.test.ts`, `renderers.test.ts`.
3. **`expansion-source.ts`** — widen `ExpansionInputs` with `expandedTools`; tool
   case returns open when `expandedTools.has(id)`.
4. **`AgentDocumentView.tsx`** — add `holdToolOpen(id)` / `releaseToolOpen(id)`
   mutators (mirror `togglePin`); pass as `onHoldToolOpen` / `onReleaseToolOpen`.
5. **`AgentDocumentVirtualList.tsx`** — accept the two callbacks; thread
   `onHoldToolOpen` to rows; add the `collapseScrolledOffTools()` scan in
   `handleScroll` (stick-gated).
6. **`DocumentRow.tsx`** — thread `onHoldToolOpen`; pass
   `heldOpen={documentState().expandedTools.has(id)}` + `onHoldOpen` to `ToolBlock`.
7. **`ToolBlock.tsx`** — delete `POST_COMPLETION_HOLD_MS` + `postCompletionHold`;
   the transition effect calls `props.onHoldOpen?.()`; `autoExpanded()` reads
   `props.heldOpen`.

## Tests

- `expansion-source.test.ts` — a completed tool in `expandedTools` resolves open;
  not in it resolves closed; pin/running still win.
- `ToolBlock.test.tsx` — `heldOpen` renders the panel in flow; without it a
  completed tool renders hidden.
- Typecheck (`tsc --noEmit`) + existing agent-pane vitest suites green.
- Live (needs running app): new tool expands, stays expanded on screen, collapses
  after scrolling off the top while streaming; loaded history starts collapsed.

## Verification commands

`npx tsc -p tsconfig.json --noEmit` · `npx vitest run <touched specs>`
