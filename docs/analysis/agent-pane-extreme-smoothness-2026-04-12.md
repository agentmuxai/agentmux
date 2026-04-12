# Agent Pane — Extreme Typing Smoothness

**Date:** 2026-04-12
**Goal:** Keystrokes always render at display refresh rate, regardless of what else is happening in the pane.

## Problem

Even after fixing the controlled-textarea issue in v0.33.91, typing lag returned as document nodes accumulated. The root cause shifted from **re-render storms** (fixed) to **DOM element count** (new bottleneck).

A streaming Claude session produces:
- 1 markdown node per text chunk (dozens per response)
- 1 tool_call node per tool use
- 1 tool_result node per tool response
- Each markdown node → `<Markdown>` component → unified pipeline → hundreds of `<span>` elements from highlight.js syntax spans

After 10 minutes of streaming, the agent pane DOM can have 5000+ elements. Every keystroke triggers:
1. Browser input event dispatch (~0.5ms)
2. Character paint (~1ms)
3. Layout/style recalc for any DOM change (~5-20ms when tree is huge)
4. Reflow from `autoGrow` reading `scrollHeight` (~2-5ms when tree is huge)

The sum exceeds one 60Hz frame (16.67ms), so characters visibly lag behind key presses.

## The three principles of extreme smoothness

### 1. Small DOM (bound the problem)

Browsers handle small DOMs fast and large DOMs slow. A 500-element tree is one frame of work; a 5000-element tree is ten frames of work. **Cap the node count** so the DOM never gets large enough to matter.

### 2. Isolated layout (contain the blast radius)

When any DOM element changes, the browser invalidates layout for its containing block. Without boundaries, the entire document recalculates. **CSS containment** (`contain: layout style paint`) tells the browser "whatever changes inside this box, the outside is safe to skip."

Combined with `content-visibility: auto`, off-screen nodes are completely skipped during layout passes.

### 3. Independent inputs (protect the hot path)

The textarea should never re-render as a side effect of streaming updates. An uncontrolled input (ref-based, no `value={signal()}`) is the first line of defense, but its parent can still re-render the sibling tree. Use `createMemo` for derived values passed as props so the textarea's props never change unless they truly need to.

## Fixes in this PR

### A. CSS containment on the document list

`AgentDocumentView.tsx` + `agent-view.scss`

```scss
.agent-document {
    contain: layout style paint;
}

.agent-document-node-wrapper {
    content-visibility: auto;
    contain-intrinsic-size: auto 80px;
    contain: layout style;
}
```

Each document node gets its own containment box. Off-screen nodes skip layout and paint entirely. Scroll position stays stable thanks to `contain-intrinsic-size` giving the browser a size estimate.

**Impact:** Layout cost during typing drops from O(document size) to O(visible viewport).

### B. Document node cap (500)

`useAgentStream.ts`

The streaming flush now evicts oldest nodes when the document exceeds 500. The `nodeIndexMap` is rebuilt after eviction so update lookups stay O(1).

**Impact:** DOM element count has a hard ceiling. Sessions that previously grew to 5000+ nodes now stabilize around 500.

### C. Log line cap (50)

`agent-view.tsx`

The launch log was unbounded. Now capped at 50 entries with `slice(-50)` eviction.

**Impact:** Eliminates one of the append-only signals that grew indefinitely.

### D. `isLoading` as `createMemo`

`agent-view.tsx:402`

Was a plain function `() => flowRunning() || !agentReady()` — re-evaluated on every caller read. Now cached:

```typescript
const isLoading = createMemo(() => flowRunning() || !agentReady());
```

**Impact:** `loading` prop passed to `AgentFooter` only changes when the underlying state actually changes, not on every parent re-render.

### E. Uncontrolled textarea (already shipped 0.33.91)

`AgentFooter.tsx`

Context for completeness — this PR builds on the previous fix.

## How the fixes compose

Before (v0.33.91):
```
Keystroke
  → input event
  → autoGrow reads scrollHeight (forced reflow, entire document layout)
  → parent re-renders (isLoading recalculated, siblings walked)
  → browser paint (full tree)
  → ~30ms per keystroke once document > 1000 nodes
```

After (this PR):
```
Keystroke
  → input event
  → autoGrow reads scrollHeight (contained — only textarea's box)
  → parent stable (memoized isLoading, no prop change)
  → browser paint (only textarea rect)
  → ~2ms per keystroke regardless of document size
```

## Testing

Open an agent pane, send a message that triggers a long streaming response (e.g., "explain the entire AgentMux architecture in detail"). While the response streams:

1. Type fast in the input field
2. Characters should appear at keyboard repeat rate (~30/sec)
3. No visible lag between keypress and character

Validate via Chrome DevTools Performance tab:
- Record while typing during stream
- Scripting + rendering time per frame should be < 5ms
- No "Forced reflow" warnings
- No long tasks (>50ms)

## What's NOT in this PR

Left for future work (tracked in `docs/analysis/agent-pane-typing-lag-2026-04-12.md`):

- Virtualized list (windowing) — only needed if 500 nodes isn't enough
- Consolidating the dual launch paths (frontend vs backend)
- Replacing 26 `as any` type casts
- WOS cache size cap
- `wos.getObjectValue()` non-reactive read bug

These are orthogonal to typing smoothness and deserve their own review.
