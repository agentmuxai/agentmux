# SPEC: Agent Pane — New Message Enter Animation

**Date:** 2026-05-30
**Status:** Ready to implement
**Area:** `frontend/app/view/agent/styles/_document.scss` + `_document-nodes.scss`
**Goal:** New nodes that stream into the agent pane should enter with the same swift fade-in as the tool-panel collapse, instead of snapping in instantly.

---

## 1. The animation we're matching

The log panel collapse (`agent-tool-panel`) already ships a polished transition:

```scss
// frontend/app/view/agent/styles/_document-nodes.scss:275-279
.agent-tool-panel {
    transition:
        max-height 120ms cubic-bezier(0.4, 0, 0.2, 1),
        padding    120ms cubic-bezier(0.4, 0, 0.2, 1),
        margin     120ms cubic-bezier(0.4, 0, 0.2, 1),
        opacity    100ms ease-out;
}
```

Duration: **120 ms**. Easing: **Material's standard cubic-bezier(0.4, 0, 0.2, 1)**. Properties: structural geometry + opacity. This is the target feel.

---

## 2. Problem

New agent-pane rows — tool blocks, markdown chunks, user messages, bash outputs — appear
instantly. There is no enter animation on `.agent-document-row` or the streaming buffer rows.
The snap is especially visible during a fast-streaming response where multiple nodes land in
quick succession; each pops into existence abruptly.

---

## 3. Architecture of the streaming buffer

New nodes during an active turn land in `.agent-document-streaming-buffer`, a plain flex
column in normal document flow (no virtualization). They are **not** virtualized while
streaming, so CSS transition on mount is safe — the elements ARE in the DOM at paint time.

**Note on short documents:** `partitionForVirtualization` places the trailing ≤50 nodes
in `.agent-document-streaming-buffer` (not the virtualizer) when the document is ≤50
nodes total. This means history rows for a short conversation also land in the streaming
buffer on open/restore — they must NOT animate. §4 handles this via a JS `[data-animate]`
gate that is absent during the initial history load and only enabled after.

Once the turn ends and the virtualizer absorbs them, rows are re-inserted as absolutely-
positioned tiles inside `.agent-document-virtualizer`. `loadOlder` rows also use the
virtualizer. Those paths should NOT animate on virtualized re-insert (that would fire every
time the user scrolls a row into the virtual window).

---

## 4. Implementation

### 4.1 Transition + @starting-style (not @keyframes)

`@starting-style` fires ONLY when an element receives its first computed styles (DOM mount
or `display:none`→visible), not when a CSS rule becomes newly applicable to an
already-mounted element. This is the key property that makes the history-gate safe.

```scss
// _document.scss — inside .agent-view
.agent-document-streaming-buffer[data-animate] .agent-document-row {
    opacity: 1;
    transform: translateY(0);
    transition: opacity 120ms cubic-bezier(0.4, 0, 0.2, 1),
                transform 120ms cubic-bezier(0.4, 0, 0.2, 1);
}

@starting-style {
    .agent-document-streaming-buffer[data-animate] .agent-document-row {
        opacity: 0;
        transform: translateY(4px);
    }
}
```

### 4.2 History gate — `[data-animate]` on the streaming buffer container

The streaming buffer div starts WITHOUT `[data-animate]`. It is added only after the
initial history load completes AND a forced browser style-resolution has occurred for the
already-mounted rows:

```ts
// AgentDocumentVirtualList.tsx
createEffect(() => {
    if (animateEnabled() || !props.viewState.historyReady()) return;
    void scrollRef?.scrollTop; // force synchronous style/layout flush (reagent P1 fix)
    setAnimateEnabled(true);
});
```

```tsx
<div class="agent-document-streaming-buffer" data-animate={animateEnabled() || undefined}>
```

The forced reflow (`void scrollRef.scrollTop`) is critical: `@starting-style` fires on the
element's **first style resolution** (at paint time, AFTER all microtasks). Without the
reflow, microtask-deferred setters would add `[data-animate]` before that resolution —
meaning history rows' first resolution sees the attribute and they animate anyway.

`historyReady()` is a signal in `AgentViewState` set by `useHistoryPagination` via a
`registerHistoryReadyCallback` prop on `AgentDocumentView`, bridged through `agent-view.tsx`.
It is set at every terminal point of the initial load:
- Snapshot restore (`HistoryRestored`)
- NDJSON load complete (`HistoryLoaded`)
- Empty document (total=0 fast-exit)
- Load failure (`InitFailed` — fail-open)

For **empty/new conversations**: `historyReady()` fires immediately with no rows in the
buffer. The forced reflow is a no-op. The first streaming row mounts WITH `[data-animate]`
present and animates correctly.

### 4.3 Reduced-motion override

```scss
@media (prefers-reduced-motion: reduce) {
    .agent-document-streaming-buffer[data-animate] .agent-document-row {
        transition: none;
    }
}
```

---

## 5. What does NOT animate

| Node path | Animated? | Reason |
|---|---|---|
| Streaming rows in `.agent-document-streaming-buffer` | ✅ Yes | New DOM mount after `[data-animate]` is present |
| History rows (any document length) in streaming buffer | ❌ No | `[data-animate]` absent during initial load; forced reflow commits their first style resolution before it is added |
| First row in a new/empty conversation | ✅ Yes | `historyReady()` fires immediately → `[data-animate]` added before first streaming row arrives |
| `loadOlder` rows prepended to virtualizer | ❌ No | Virtualizer rows, not in streaming buffer |
| Tool-panel expand/collapse | ✅ Already (existing) | Separate `max-height` transition on `.agent-tool-panel` |
| Pane-level open/close reflow | ❌ Out of scope | See `ANALYSIS_PANE_OPEN_CLOSE_ANIMATION_2026_05_29.md` |

---

## 6. Edge cases

**Fast-streaming batches (many nodes in one RAF):** Each row gets the same 120ms keyframe
starting from its own mount frame. SolidJS batches DOM writes in one microtask flush, so
all nodes in a batch mount at approximately the same timestamp — they animate in unison,
not in a staircase, which looks clean.

**Very short nodes (single-line tool summary):** 120ms at `opacity: 0 → 1` with a 4px
lift is subtle enough that single-line nodes don't look like they're "falling in"; they
just appear with a brief fade.

**Existing nodes during re-mount (hot reload / Vite HMR):** HMR replaces the full component
tree, so all `.agent-document-row` elements re-mount and will re-animate. Acceptable in
dev; not a production concern.

**`prefers-reduced-motion: reduce` users:** No animation at all — rows snap in as today.

---

## 7. Files to change

| File | Change |
|---|---|
| `frontend/app/view/agent/styles/_document.scss` | Add `@keyframes agent-node-enter` and the two rulesets (§4.1–4.3) inside `.agent-view` |

That's it — one file, ~15 lines of CSS.

---

## 8. Acceptance criteria

- [ ] New streaming nodes (tool blocks, markdown, user messages, bash output) fade in over
      ~120ms with a subtle 4px upward motion.
- [ ] History rows replayed via `loadOlder` do NOT animate.
- [ ] `prefers-reduced-motion: reduce` users see instant appearance (no animation).
- [ ] No regression on tool-panel expand/collapse animation.
- [ ] No layout shift or scroll-position jump during the enter animation.
- [ ] Passes visual check in `task dev` with a real Claude-stream session.

---

## 9. Related

- `docs/analysis/ANALYSIS_PANE_OPEN_CLOSE_ANIMATION_2026_05_29.md` — pane reflow animation
  (separate, harder problem; this spec is independent of it)
- `frontend/app/view/agent/styles/_document-nodes.scss:253-298` — existing tool-panel
  collapse animation (the reference implementation)
- `frontend/layout/lib/tilelayout.scss:201-231` — placeholder enter/exit (another
  reference for `@keyframes` enter patterns in this codebase)
