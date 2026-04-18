# Node Timestamp Hover

**Date:** 2026-04-15
**Status:** Ready to implement

---

## Goal

Every document node gets a capture timestamp. Hovering any node reveals a
floating popover showing the time in `HH:MM:SS.M` format. This also serves
as a lightweight diagnostic for out-of-order message delivery.

---

## Current State

| Node type        | Has `timestamp`? | Rendered? |
|------------------|-----------------|-----------|
| `user_message`   | Yes             | No        |
| `agent_message`  | Yes (ms epoch)  | Yes — `toLocaleTimeString()` in AgentMessageBlock, no hover |
| `markdown`       | No              | —         |
| `tool`           | No              | —         |
| `section`        | No              | —         |
| `subagent_link`  | No              | —         |

---

## Changes Required

### 1. Add `timestamp` to all node types — `types.ts`

Add `timestamp: number` (Unix ms) to `MarkdownNode`, `ToolNode`,
`SectionNode`, and `SubagentLinkNode`. Field is optional (`timestamp?:
number`) so old history records without the field keep working.

`AgentMessageNode` and `UserMessageNode` already have non-optional
`timestamp: number` — no change needed.

### 2. Stamp nodes at creation — `useAgentStream.ts`

Every `pendingNew.push({...})` call adds `timestamp: Date.now()` to the
node. This covers:
- `markdown` nodes from text/thinking deltas
- `tool` nodes on `tool_call` events
- `section` nodes
- `subagent_link` nodes
- The agenthealth nodes are removed (v0.33.193)

`AgentMessageNode` and `UserMessageNode` already use
`event.timestamp || Date.now()`.

### 3. Hover popover — shared component

New file: `frontend/app/view/agent/components/NodeTimestamp.tsx`

```tsx
// Renders nothing visible inline.
// On parent hover (via CSS :hover on .doc-node), shows a small floating
// pill in the top-right corner of the node row.
```

Implementation:
- A `<Show when={props.timestamp != null}>` wrapper
- Formats the timestamp as `HH:MM:SS.M` (local time, tenths of a second):
  ```typescript
  function formatTime(ms: number): string {
      const d = new Date(ms);
      const h = String(d.getHours()).padStart(2, "0");
      const m = String(d.getMinutes()).padStart(2, "0");
      const s = String(d.getSeconds()).padStart(2, "0");
      const t = Math.floor(d.getMilliseconds() / 100); // tenths
      return `${h}:${m}:${s}.${t}`;
  }
  ```
- Renders a `<span class="node-ts">` absolutely positioned to top-right
  of the nearest `position: relative` ancestor (the node row wrapper)
- Visibility controlled entirely by CSS: `opacity: 0` by default,
  `.doc-node:hover .node-ts { opacity: 1 }` — no JS state needed

### 4. Add `.doc-node` wrapper + CSS — `AgentDocumentView.tsx` + `agent-view.scss`

`DocumentNodeRenderer` wraps each rendered node in:
```tsx
<div class="doc-node">
  {/* existing node JSX */}
  <NodeTimestamp timestamp={node.timestamp} />
</div>
```

CSS in `agent-view.scss`:
```scss
.doc-node {
    position: relative;

    .node-ts {
        position: absolute;
        top: 4px;
        right: 6px;
        font-size: 10px;
        font-family: var(--fixed-font);
        color: var(--secondary-text-color);
        background: var(--panel-bg-color);
        border: 1px solid var(--border-color);
        border-radius: 4px;
        padding: 1px 5px;
        pointer-events: none;
        opacity: 0;
        transition: opacity 80ms ease;
        white-space: nowrap;
        z-index: 10;
    }

    &:hover .node-ts {
        opacity: 1;
    }
}
```

---

## What This Does NOT Change

- No inline timestamp rendering in the node body (replaces the existing
  `toLocaleTimeString()` in `AgentMessageBlock` — that rendered always-on,
  we move it to the hover pill instead)
- No sort/reorder logic — timestamps are diagnostic only
- No persistence change — timestamps are ephemeral (live session only);
  history replay nodes without a timestamp simply show no pill

---

## Files Touched

| File | Change |
|------|--------|
| `frontend/app/view/agent/types.ts` | Add `timestamp?: number` to `MarkdownNode`, `ToolNode`, `SectionNode`, `SubagentLinkNode` |
| `frontend/app/view/agent/useAgentStream.ts` | Stamp `Date.now()` on every `pendingNew.push` |
| `frontend/app/view/agent/components/NodeTimestamp.tsx` | New component |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | Wrap each node in `.doc-node`, render `<NodeTimestamp>` |
| `frontend/app/view/agent/components/AgentMessageBlock.tsx` | Remove existing always-on timestamp (superseded by pill) |
| `frontend/app/view/agent/agent-view.scss` | `.doc-node` + `.node-ts` styles |
