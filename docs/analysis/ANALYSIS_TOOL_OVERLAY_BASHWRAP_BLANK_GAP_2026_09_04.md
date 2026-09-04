# Analysis: "Thinking… → blank → output" gap survived the bashwrap-hiding fix

**Date:** 2026-09-04
**Status:** implemented
**Author:** Agent5

## User's request (verbatim, for traceability)

> recently we were working on backward scrolling on the agent pane. get latest github main code .. the last fix was how Thinking... and the [basshwrap] caused it to move backward, but the agent removed bashwrap, it is still going backward because it goes from Thinking... to blank, then shows the output. we need to skip that middle part

## This is a distinct bug from the broader "scroll oscillation" investigation

`docs/analysis/ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md` and its
two follow-up `FINDINGS_*` docs analyze a **different, still-open** problem
(un-eased shrink-then-regrow of the outer scroll container across several
independent DOM-shape-swap mechanisms — issues #2648/#2718, not touched
here). This document is about a narrower, already-fixed-once regression:
PR #2948 (`ed2dd61`, `fix(agent-pane): hide bashwrap's internal
starting-chunk from the tool log`) hid the literal `[bashwrap] starting: N
chars` debug text from the tool-call log panel, but left a gap that
produces the same *symptom* (a visible non-output state between "Thinking…"
and real output) via a different, more subtle mechanism.

## Root cause

`ToolOverlayLog.tsx`'s render-branch selector:

```ts
const isStreaming = () => props.node.log?.open === true;
const hasChunks = () => (props.node.log?.chunks?.length ?? 0) > 0;   // BEFORE THE FIX
const hasResult = () => props.node.result != null;
```

```tsx
<Switch>
    <Match when={isStreaming() && hasChunks()}>
        <ChunkList chunks={chunks()} />
    </Match>
    ...
    <Match when={!hasChunks() && !hasResult()}>
        <ToolOverlayResult node={props.node} dispatchMatch={props.dispatchMatch} />
    </Match>
</Switch>
```

`ChunkList` itself already calls `dropBashwrapStartingChunk()` (PR #2948)
before rendering — so once a Bash tool call is `running` with only its
always-first `[bashwrap] starting: N chars` chunk received, `ChunkList`
correctly renders **nothing**. But `hasChunks()` counts the *raw*
`log.chunks.length`, before that filter — so with exactly one (filtered)
chunk present, `hasChunks()` still reads `true`, `isStreaming() &&
hasChunks()` still matches, and the `<Switch>` still routes to `ChunkList`.
The result: a visibly empty box, instead of falling through to the
`!hasChunks() && !hasResult()` branch, whose `ToolOverlayResult` renders:

```tsx
<div class="agent-tool-loading">
    <span class="agent-tool-spinner">⏳</span> Thinking...
</div>
```

— literally the same "Thinking…" text the Working row (`AgentFooter.tsx`)
already shows. PR #2948 fixed what `ChunkList` renders once it's the
*active* branch; it never touched which branch gets chosen, so the
branch-selection gate still disagreed with what the chosen branch would
actually show.

Combined with `ToolBlock.tsx`'s `autoExpanded()` (auto-expands the panel
the instant `status === "running"`, independent of whether there's
anything to show yet), the visible sequence became: "Thinking…" (Working
row) → tool panel auto-expands into an empty box (blank) → real output
replaces the empty box. Exactly the "Thinking… → blank → output" the user
reports, and exactly the middle step they asked to skip.

## Fix

One-line change, same file:

```ts
const hasChunks = () => dropBashwrapStartingChunk(props.node.log?.chunks ?? []).length > 0;
```

Now an all-bashwrap-marker chunk list correctly reads as "nothing to show
yet," the `<Switch>` falls through to `ToolOverlayResult`'s "⏳
Thinking..." placeholder, and because that placeholder's text is identical
to the Working row's own "Thinking…" phrase, the visible transition is:
"Thinking…" (Working row) → "⏳ Thinking..." (tool panel, same words) →
real output. No blank box, no distinguishable middle state at all until
real content exists.

Deliberately scoped to `ToolOverlayLog.tsx` only. `PersistentShellBlock.tsx`
has the identical `dropBashwrapStartingChunk` filtering (also from #2948)
but its panel only expands on explicit user pin (`expanded() =>
props.pinned`), never automatically on tool start — so it can't produce
this auto-expand-into-blank sequence and doesn't need the equivalent fix.

## Verification

Extended the existing `ToolOverlayLog.test.tsx` "hides bashwrap's internal
starting-chunk" describe block with a test that specifically distinguishes
"correctly blank" from "wrongly blank instead of the Thinking placeholder"
(the pre-existing tests in that block don't — both the bug and the fix
produce zero `.agent-tool-log-line` rows, so only a test asserting the
placeholder's *presence* catches the regression). Falsified by reverting
the fix in isolation and confirming exactly that one new test fails (11/12
still pass); restored, diff clean. Full frontend suite: 3620/3620 passing.
`tsc --noEmit` clean.

## Files

| File | Role |
|---|---|
| `frontend/app/view/agent/components/ToolOverlayLog.tsx` | `hasChunks()` — the fixed gate; `ToolOverlayResult`'s "⏳ Thinking..." fallback |
| `frontend/app/view/agent/components/output-cap.ts` | `dropBashwrapStartingChunk` — the render-layer filter `ChunkList` already applied |
| `frontend/app/view/agent/components/ToolBlock.tsx` | `autoExpanded()` — why the panel is visible at all during this window |
| `frontend/app/view/agent/components/PersistentShellBlock.tsx` | Has the same filter, not the same bug (manual-pin-only expansion) |
| `docs/analysis/ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md` | The broader, still-open, unrelated scroll-oscillation investigation |
