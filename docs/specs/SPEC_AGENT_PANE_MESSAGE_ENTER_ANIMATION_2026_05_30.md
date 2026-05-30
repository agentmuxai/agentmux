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
streaming, so CSS `@keyframes` / transition on mount is safe — the elements ARE in the DOM at
paint time.

Once the turn ends and the virtualizer absorbs them, rows are re-inserted as absolutely-
positioned tiles inside `.agent-document-virtualizer`. Replayed history rows also enter via
the virtualizer. Those paths are covered by §5 (reduced-motion guard) and should NOT
animate on virtualized re-insert (that would fire every time the user scrolls a row into
the virtual window).

---

## 4. Implementation

### 4.1 Keyframe definition

Add to `_document.scss` inside `.agent-view`:

```scss
@keyframes agent-node-enter {
    from {
        opacity: 0;
        transform: translateY(4px);
    }
    to {
        opacity: 1;
        transform: translateY(0);
    }
}
```

`4px` translateY keeps it subtle — enough to convey direction (downward stream) without
feeling like a full slide-in. Matches the "swift" character of the tool-panel collapse.

### 4.2 Apply to streaming-buffer rows only

```scss
// _document.scss — inside .agent-view
.agent-document-streaming-buffer .agent-document-row {
    animation: agent-node-enter 120ms cubic-bezier(0.4, 0, 0.2, 1) both;
}
```

Scoped to `.agent-document-streaming-buffer` — not `.agent-document-virtualizer` — so
the animation fires ONLY when a new node mounts into the live streaming region. Virtualized
rows that scroll in/out of the DOM window are NOT affected.

`animation-fill-mode: both` ensures the node starts at `opacity: 0` even before the first
frame if there's any scheduling jitter.

### 4.3 Reduced-motion override

```scss
@media (prefers-reduced-motion: reduce) {
    .agent-document-streaming-buffer .agent-document-row {
        animation: none;
    }
}
```

Required. Respects the OS accessibility setting; mirrors the existing reduced-motion guard
in `tilelayout.scss`.

### 4.4 No JS changes required

This is CSS-only. The SolidJS component tree, the virtualizer, the document store, and
the streaming reducer are all untouched.

---

## 5. What does NOT animate

| Node path | Animated? | Reason |
|---|---|---|
| Streaming rows in `.agent-document-streaming-buffer` | ✅ Yes | New DOM mount |
| History rows replayed via virtualizer | ❌ No | `.agent-document-virtualizer` excluded from selector |
| `loadOlder` rows prepended to virtualizer | ❌ No | Same — appear above the viewport, no visible flash |
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
