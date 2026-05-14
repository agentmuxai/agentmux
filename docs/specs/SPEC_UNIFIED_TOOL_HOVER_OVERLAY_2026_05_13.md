# Spec: Unified tool-block hover overlay (no double-popup)

**Status:** Spec (no implementation yet)
**Owner:** AgentA
**Date:** 2026-05-13
**Driving requirement:** "I see two separate popups now — the time strip and the live-log overlay. Merge them into one. Put the time at the bottom of the live-log."

---

## 1. TL;DR

Today a hover over a tool block surfaces two visually-distinct UI elements:

- **`NodeHoverStrip`** — row-level strip with the timestamp + bookmark + expand buttons. CSS-driven (`.agent-document-node-wrapper:hover`), appears whenever the row is hovered, regardless of node type.
- **`ToolBlockOverlay`** — tool-only portal overlay with status header, log body, and action bar (Open in pane / Open in window / + New agent here). JS-driven (`hovering()` signal + 150ms enter delay).

Both fire on the same hover. The user sees two stacked tooltips with overlapping affordances (bookmark exists on both; "expand" on the strip is now redundant with hover-expand on the block itself).

This spec collapses the two surfaces for tool blocks into a single overlay:

- The hover strip stays for **non-tool** nodes (markdown, agent_message, etc.).
- For tool nodes the strip is **suppressed** entirely.
- The tool overlay grows a **footer slot** that carries the timestamp + bookmark in a compact strip styled to match the header.
- The expand button disappears (hover-expand subsumes it; click-to-pin is the persistence affordance).

End result: hovering a tool row shows exactly one popup, anchored to the block, with status on top, live-log in the middle, and timestamp + actions on the bottom.

---

## 2. Today's state

### Two hover surfaces

```
┌─ document row ────────────────────────────────────────┐
│  $ ls -la                            ⏳ 0.3s          │   ← collapsed tool block (always visible)
└──────────────────────────────────────────────────────┘
       ▲ hover here
       │
       ├─→ NodeHoverStrip pops in beside the row:
       │     [Tue May 13 02:14 AM] [⊞ Expand] [🔖]
       │
       └─→ ToolBlockOverlay pops in below the row (portal):
             ┌──────────────────────────────────────────┐
             │ ✓ Bash — ls -la                  0.3s ok │   ← header
             ├──────────────────────────────────────────┤
             │ total 24                                 │   ← log body
             │ drwxr-xr-x  3 area54  ...                │
             │ ...                                      │
             ├──────────────────────────────────────────┤
             │ [Open in pane] [+ New agent here] [↗]    │   ← actions
             └──────────────────────────────────────────┘
```

Both fire on the same physical hover, anchored to the same row. The strip is row-level; the overlay is portal'd to `document.body` so it escapes the document container's paint containment. They appear at different positions and at different scales (strip is one line; overlay is a panel).

### Affordance overlap

| Surface | Has | Not on the other |
|---|---|---|
| `NodeHoverStrip` | timestamp, bookmark, expand button | — |
| `ToolBlockOverlay` | status, log body, action bar (open in pane / window / new agent) | timestamp |

The expand button on the strip is **already redundant** post-PR #825 — hover-expand makes the overlay appear automatically.
The bookmark icon appears on BOTH surfaces because the overlay's action bar also has one.

---

## 3. Goals

1. **One popup on hover for tool blocks.** No competing tooltips, no double-anchor.
2. **Timestamp lives at the bottom of the tool overlay**, formatted the same way the strip currently does it (`Tue May 13, 2:14 AM` with localized weekday/date/12-hour-AM-PM, year added when ≥7 days old).
3. **Bookmark stays accessible** — only once, in the overlay's action bar.
4. **NodeHoverStrip continues to work for non-tool nodes** (markdown, agent_message). Those still want a timestamp + bookmark on row-hover.
5. **No regression in click-to-pin or scroll-anchored overlay positioning**.

### Non-goals

- No animation / fade work in this spec. Visibility is binary on hover state.
- No restructuring of the overlay's header / log / action-bar three-slot layout.
- No changes to non-tool node hover behavior.

---

## 4. Design

### A. `ToolBlockOverlay` grows a footer

Currently a three-slot layout (header, log, action-bar). Add a **fourth slot below the action bar** — the metadata footer:

```
┌──────────────────────────────────────────────────────┐
│ ✓ Bash — ls -la                          0.3s ok     │   ← header (existing)
├──────────────────────────────────────────────────────┤
│ total 24                                             │
│ drwxr-xr-x  3 area54  ...                            │   ← log body (existing)
│ ...                                                  │
├──────────────────────────────────────────────────────┤
│ [Open in pane] [Open in window] [+ New agent here]   │   ← action bar (existing)
├──────────────────────────────────────────────────────┤
│ Tue May 13, 2:14:32 AM                          🔖   │   ← NEW: metadata footer
└──────────────────────────────────────────────────────┘
```

**Footer contents:**

- **Left:** localized timestamp via the same `formatLocalized()` from `NodeHoverStrip`. Wrapped in `<time datetime="...">` for a11y.
- **Right:** bookmark toggle. Replaces the bookmark in the action bar — single affordance, no duplication.

**Footer style:**

- Single line, ~24px tall, secondary text color
- `border-top` to match the other slot dividers
- Bookmark button styled smaller than action-bar buttons (icon-only, no label)

### B. Suppress `NodeHoverStrip` for tool nodes

In `frontend/app/view/agent/components/AgentDocumentView.tsx` (or wherever the strip is rendered), gate on `node.type !== "tool"`:

```tsx
<Show when={node.type !== "tool"}>
    <NodeHoverStrip
        timestamp={node.timestamp}
        nodeId={node.id}
        isBookmarked={...}
        onBookmark={...}
        canExpand={...}
        onExpand={...}
    />
</Show>
```

The strip continues to render for markdown / agent_message / user_message nodes.

### C. Bookmark wiring

`ToolBlockOverlay`'s existing `isBookmarked` + `onBookmark` props already cover this — the footer just relocates the visual control. No prop changes.

### D. Tool block's inline duration

Keep the inline `(0.3s)` next to the tool name in the collapsed row. It's not "the timestamp" — it's the duration measurement that gives the user a glanceable signal of "this tool took 0.3s" even without hover. The footer's timestamp answers a different question ("when did this run"). Both have their place.

---

## 5. Hover delay (the user's other question)

Today: 150ms enter delay, 300ms leave delay (per `docs/specs/tool-collapse.md`).

The 150ms is the spec'd value — "prevents flicker on scroll-through". Rationale:

- A user scrolling rapidly past 10 tool blocks shouldn't see 10 overlays flash in/out.
- 150ms is the lower bound where most quick scroll-throughs are filtered out.
- It's also the standard Tailwind / Material design "expansion" delay.

If the user finds 150ms too slow on a deliberate hover: lower it to **80ms** (the Slack / VSCode tooltip default). Anything below ~50ms creates visual jitter during scroll.

**Recommendation:** keep 150ms but make it configurable later via a settings entry. For this spec we ship 150ms unchanged.

---

## 6. Implementation steps

### Step 1 — `ToolBlockOverlay` footer

`frontend/app/view/agent/components/ToolBlockOverlay.tsx`:

- Add a `timestamp?: number` prop (Unix ms).
- Render a new `.agent-tool-overlay-footer` `<div>` below the action bar when `timestamp != null`.
- Move the bookmark from the action bar (`ToolOverlayActions`) into the footer.
- `ToolOverlayActions` keeps the branching actions (Open in pane / Open in window / + New agent here) only.

### Step 2 — `ToolBlock` passes timestamp through

`frontend/app/view/agent/components/ToolBlock.tsx`:

- Add `timestamp?: number` to `ToolBlockProps`.
- Forward to `ToolBlockOverlay`.

### Step 3 — caller plumbs timestamp

Find the parent that renders `ToolBlock` (likely `DocumentNodeRenderer`) and pass `node.timestamp ?? node.startedAt ?? Date.now()` — whichever field the `ToolNode` carries. (Verify against types.ts during impl.)

### Step 4 — Suppress strip for tools

Wherever `NodeHoverStrip` is rendered in the document view, wrap the render in `<Show when={node.type !== "tool"}>`.

### Step 5 — Styles

`frontend/app/view/agent/styles/_tool-overlay.scss` (or wherever the overlay slots are styled):

- `.agent-tool-overlay-footer` — flex row, `justify-content: space-between`, `border-top: 1px solid var(--border-color)`, `padding: var(--space-1-5) var(--space-2)`, `color: var(--secondary-text-color)`, `font-size: 11px`.
- `.agent-tool-overlay-footer time` — left aligned.
- `.agent-tool-overlay-footer button` (the bookmark) — right aligned, icon-only, no border.

### Step 6 — Tests

- `ToolBlockOverlay.test.tsx`: renders the footer when `timestamp` prop set; does not render it when absent.
- `AgentDocumentView.test.tsx` (or similar): for a tool node, `NodeHoverStrip` is not rendered; for a markdown node it is.

---

## 7. Risks + open questions

| Risk | Mitigation |
|---|---|
| Bookmark icon migrating from action bar → footer is a behavior change for muscle memory. | Move only — same icon, same action. Document in the PR body. |
| Some tools don't have a timestamp on the `ToolNode` (older nodes from before the timestamp field was added). | Gate the footer on `timestamp != null` — no footer at all rather than "Unknown date". |
| The non-tool strip still appears for tool nodes' adjacent markdown nodes — could feel inconsistent. | Acceptable. The markdown row IS a different node; users will understand the strip is per-row. |
| Two overlays might briefly overlap during a hover-leave-hover-enter (strip on adjacent row, overlay on this row). | Out of scope — that's the document view's normal behavior. |

### Open question

Should the **footer also show the `tool_use_id`** for power users who want to grep `~/.agentmux/logs/bashwrap-debug.log`? Probably yes but small + dim. Decide during impl.

---

## 8. PR scope

Single PR. Touches:

- `frontend/app/view/agent/components/ToolBlockOverlay.tsx` (+~30 lines)
- `frontend/app/view/agent/components/ToolBlock.tsx` (+~5 lines for prop forwarding)
- `frontend/app/view/agent/components/ToolOverlayActions.tsx` (–~10 lines for bookmark removal)
- `frontend/app/view/agent/components/AgentDocumentView.tsx` (+~3 lines for Show gate)
- `frontend/app/view/agent/styles/_tool-overlay.scss` (+~15 lines for footer styles)
- Tests as listed in §6 step 6

Estimated diff: ~60 lines net. Single PR, single review cycle.
