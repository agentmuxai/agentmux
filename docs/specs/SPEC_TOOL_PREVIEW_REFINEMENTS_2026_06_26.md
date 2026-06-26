# SPEC — Tool Preview Refinements: Word-wrap + Independent Zoom

**Date:** 2026-06-26  
**Status:** Analysis complete — ready to implement

---

## Feature 1: Remove distorting word-wrap from tool output

### Symptom

Commands like `find | head`, `ls -la`, `git log --oneline`, or any tool that
produces fixed-width columnar output look distorted in the tool preview panel.
Lines that are meant to be read as a unit get wrapped mid-token at arbitrary
column boundaries, destroying the tabular alignment the command was designed to
produce.

### Root cause

`_tool-overlay-portal.scss:47-48`:
```scss
.agent-tool-log-line {
    white-space: pre-wrap;   /* preserves whitespace but wraps at container edge */
    word-break: break-word;  /* breaks long tokens — intended for paths/base64 */
    ...
}
```

`pre-wrap` keeps newlines but still wraps long lines. `break-word` then forces
a word-boundary break anywhere a token hits the container edge. Together, this
causes `find | head` output like:

```
./frontend/app/view/agent/compon
ents/ToolBlock.tsx
```

instead of the intended:

```
./frontend/app/view/agent/components/ToolBlock.tsx
```

The container (`agent-tool-log-body` in `_tool-overlay-portal.scss`) already has
`overflow-y: auto` for vertical scrolling. It does NOT have `overflow-x: auto`,
so horizontal overflow silently wraps instead of producing a scrollbar.

### Fix

Two changes:

**`_tool-overlay-portal.scss`** — change `.agent-tool-log-line`:
```scss
.agent-tool-log-line {
    white-space: pre;         /* was: pre-wrap — preserve whitespace, no soft wrap */
    overflow-wrap: normal;    /* was: word-break: break-word — let container clip/scroll */
    ...
}
```

And enable horizontal scroll on the log body so long lines don't clip:
```scss
.agent-tool-log-body {
    overflow-x: auto;         /* add: horizontal scroll for long lines */
    overflow-y: auto;         /* already present */
    ...
}
```

**Why `pre` instead of `pre-wrap`?** Tool output (bash, grep, find, git) is
already newline-terminated by the subprocess. `pre` preserves those newlines
without injecting additional wrap points. Text that genuinely needs word-wrap
(e.g. the `content` result of a `Read` tool returning a prose document) is
rendered by separate structured result viewers (`BashResult`, `ReadResult`, etc.
in `ToolOverlayLog.tsx:256-336`) that have their own CSS — they don't use
`.agent-tool-log-line`.

**Edge case: very long single tokens (base64, minified JS).** These won't wrap
anymore. They'll produce a horizontal scrollbar on the log body, which is the
correct UX — the user can scroll rather than seeing a garbled column layout. If
this becomes a concern, a separate class (`.agent-tool-log-line--wrap`) can be
applied to specific renderers that need wrap behavior.

**Files to change:**

| File | Change |
|---|---|
| `frontend/app/view/agent/styles/_tool-overlay-portal.scss` | `.agent-tool-log-line`: `white-space: pre`, `overflow-wrap: normal` |
| `frontend/app/view/agent/styles/_tool-overlay-portal.scss` | `.agent-tool-log-body`: add `overflow-x: auto` |

---

## Feature 2: Independent zoom for tool preview

### Desired behavior

- **Hovering over the tool preview panel** + `Ctrl+Scroll`: zooms only the tool
  preview's font size. The rest of the agent pane (conversation, composer, header)
  stays at its current zoom.
- **Ctrl+Scroll anywhere else** in the agent pane: works as today — zooms the
  entire pane via `block.meta["term:zoom"]`.
- The tool preview zoom is **ephemeral per-session** (not persisted to block
  meta). Justification: tool previews open and close frequently; persisting a
  separate zoom key would accumulate stale meta. A signal resets on close.
- Zoom range: 0.7x–2.0x (tighter floor than pane zoom since tool output is
  already monospace and compact).
- Visual feedback: reuse the existing `ZoomIndicator` component but with a
  label like "Preview 120%".

### Current architecture

`agent-view.tsx:1014` applies zoom to the root div:
```tsx
style={{ zoom: zoomFactor(), "--agent-pane-zoom": String(zoomFactor()) }}
```

`zoomFactor` reads `block.meta["term:zoom"]` via a memo. The tool overlay
(`ToolBlockOverlay`, `ToolOverlayLog`) inherits this via CSS `zoom` cascade.

`ToolBlock.tsx` renders the overlay as a child of the document flow. The tool
log body (`agent-tool-log-body`) is a scrollable div inside the overlay — the
logical interception point for an independent wheel handler.

### Implementation plan

#### Step 1 — Tool-preview zoom signal

In `ToolBlock.tsx` (or a shared context), create a local signal:

```typescript
// Inside ToolBlock or a wrapping context
const [toolPreviewFontScale, setToolPreviewFontScale] = createSignal(1.0);

const clampToolZoom = (v: number) => Math.min(2.0, Math.max(0.7, v));
const TOOL_ZOOM_STEP = 0.05;

const handleToolWheelZoom = (e: WheelEvent) => {
    if (!e.ctrlKey) return;
    e.preventDefault();
    e.stopPropagation(); // prevent the pane-level wheel handler from also firing
    const delta = e.deltaY > 0 ? -TOOL_ZOOM_STEP : TOOL_ZOOM_STEP;
    setToolPreviewFontScale(prev => clampToolZoom(prev + delta));
    showZoomIndicator(`Preview ${Math.round(toolPreviewFontScale() * 100)}%`);
};
```

#### Step 2 — Apply scale to the log body

In `ToolOverlayLog.tsx`, pass `toolPreviewFontScale` down and apply it:

```tsx
<div
    class="agent-tool-log-body"
    style={{ "font-size": `${toolPreviewFontScale() * 100}%` }}
    onWheel={handleToolWheelZoom}
>
```

Using `font-size` rather than CSS `zoom` because:
- `font-size` scales text without affecting the container's layout box (the
  overlay stays at the same pixel height; only content inside scales).
- `zoom` would scale the container itself, shifting surrounding layout.
- The log body is already `overflow: auto` — the scrollbar adjusts automatically
  as font-size changes.

#### Step 3 — Event isolation

The pane-level `Ctrl+Wheel` handler lives in `agent-view.tsx` (via the zoom
framework in `zoom.win32.ts` / `zoom.darwin.ts`). To prevent double-firing:

- Call `e.stopPropagation()` in `handleToolWheelZoom` — the pane's wheel
  listener registered on the outer container won't see the event.
- The pane zoom (`block.meta["term:zoom"]`) is unaffected.

#### Step 4 — Reset on close

Since the overlay closes when the tool block collapses, the signal is naturally
scoped to the component's lifetime. No explicit reset needed — SolidJS disposes
the signal on component cleanup.

If `ToolBlock` is kept mounted (hidden via `display: none`) for performance,
add an explicit reset:
```typescript
createEffect(() => {
    if (!expanded()) setToolPreviewFontScale(1.0);
});
```

#### Step 5 — Keyboard zoom (optional, phase 2)

For keyboard `Ctrl++`/`Ctrl+-` when focus is inside the tool preview, the same
pattern applies: intercept keydown on the log body, call `stopPropagation`, and
update the signal. This is lower priority since the overlay doesn't capture focus
in the current design.

### Signal propagation — where to own the signal

Two options:

| Option | Tradeoff |
|---|---|
| **In `ToolBlock`**, passed down to `ToolBlockOverlay` → `ToolOverlayLog` as props | Simple, minimal — best for one use site |
| **Context via `createContext`**, consumed in `ToolOverlayLog` directly | Better if multiple overlay sub-components need the scale (e.g. a future header font) |

Recommend **props** for now — avoids context machinery for a single value.

### Files to change

| File | Change |
|---|---|
| `frontend/app/view/agent/components/ToolBlock.tsx` | Add `toolPreviewFontScale` signal + `handleToolWheelZoom`; pass scale as prop to overlay |
| `frontend/app/view/agent/components/ToolBlockOverlay.tsx` | Accept + forward `fontScale` prop to `ToolOverlayLog` |
| `frontend/app/view/agent/components/ToolOverlayLog.tsx` | Accept `fontScale` prop; apply to log body `style` + attach `onWheel` handler |

---

## Related specs

- `SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` — tool chunk delivery
- `SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md` — shell node log rendering (same log body, same fix applies)
- `docs/analysis/AGENT_PANE_REDUCER_AUDIT_2026_05_12.md` — zoom/layout invariants (INV-2)
