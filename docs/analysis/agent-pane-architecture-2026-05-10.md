# Agent pane architecture report

**Date:** 2026-05-10
**Author:** AgentA
**Trigger:** "take another deeper look into the structure of the agent pane dialog system" — investigating the reported "thinking blocks aligned horizontally / newlines clumped around tables and thinking entries" symptom.

This report walks the complete pipeline from NDJSON bytes off the CLI to pixels on the screen, then closes with the specific structural locations where the reported symptom can originate.

---

## 1. Pipeline overview

```
Claude CLI stdout (NDJSON, one event per line)
        │
        ▼
ClaudeCodeStreamParser  (frontend/app/view/agent/stream-parser.ts)
        │   parseStreamEvent(event) -> DocumentNode | null
        │   maintains currentTextNode / currentThinkingNode mutable accumulators
        ▼
agent-document-store (Solid store, pure reducer)
        │   upserts node by id; preserves array order
        ▼
useAgentStream (frontend/app/view/agent/useAgentStream.ts)
        │   wires parser output into store dispatches
        ▼
AgentViewState (frontend/app/view/agent/virtualization/state.ts)
        │   nodes, nodeIndex, stickToBottom, headAnchor, streamingNodeId
        ▼
AgentDocumentVirtualList  (hybrid virtualized + streaming buffer)
        │
        ▼
DocumentRow → per-kind block (MarkdownBlock | ToolBlock | AgentMessageBlock | …)
        │
        ▼
.agent-document → .agent-document-virtualizer + .agent-document-streaming-buffer
```

**Single source of truth:** scroll state lives in the Solid store (`stickToBottom`, `headAnchor`), never in `scrollRef.scrollTop`. The DOM is a projection.

---

## 2. DocumentNode types

Discriminated union, one shape per kind. Lives in `frontend/app/view/agent/types.ts`.

| `type` | Renderer | Notes |
|---|---|---|
| `markdown` | `MarkdownBlock` | Carries `metadata.thinking?: true` for thinking deltas; same shape, italic + low opacity styling. **Tables are rendered inside this kind.** |
| `tool` | `ToolBlock` | Collapsed-by-default one-line summary. Pinned/expanded uses a `<Portal>` to escape paint containment. |
| `agent_message` | `AgentMessageBlock` | Inter-agent messages with direction (incoming/outgoing). |
| `user_message` | `<div class="agent-user-message">` (inline) | Plain `<pre>` of message text. |
| `subagent_link` | `SubagentLinkBlock` | 56px fixed-height reference into a sub-agent. |
| `section` | inline `<h1>/<h2>/<h3>` | Section headers — no separate component. |

**Critical:** thinking is encoded as a `markdown` node with `metadata: { thinking: true }`, not as a distinct `type`. This matters for the layout investigation in §9.

---

## 3. Thinking & text accumulation

`stream-parser.ts` keeps two mutable accumulators:

```ts
private currentTextNode:     { type: "markdown"; id; content } | null
private currentThinkingNode: { type: "markdown"; id; content; metadata: { thinking: true } } | null
```

Behavior on each event:

| Event | Effect on accumulators |
|---|---|
| `text` (delta) | `currentThinkingNode = null`; create or **append `event.content`** to `currentTextNode`. Returns the same node id with growing content. |
| `thinking` (delta) | `currentTextNode = null`; create or **append `event.content`** to `currentThinkingNode`. |
| `tool_call` / `tool_result` / `agent_message` / `user_message` | Both accumulators reset to null; future text/thinking starts a fresh node. |

**Key consequence:** consecutive text deltas merge into one `markdown` node. Consecutive thinking deltas merge into one `markdown` node with the thinking flag. Switching kinds cuts a fresh node.

**Newline behavior:** the parser appends `event.content` verbatim with **no separator injection**. If the upstream stream emits two thinking deltas without a trailing newline in the first delta, the resulting accumulated `content` has no paragraph break between them. This is one of the candidate root causes for "clumped newlines" — see §9.

---

## 4. Hybrid virtualization (the core design)

`AgentDocumentVirtualList.tsx` partitions the document into two halves on every render:

```
┌─ scrollRef (.agent-document) ─ display: flex, flex-direction: column ─┐
│                                                                        │
│  ┌─ .agent-document-virtualizer (position: relative) ───────────────┐ │
│  │  height = virtualizer.getTotalSize()                             │ │
│  │  rows: position: absolute; transform: translateY(start)          │ │
│  │  TanStack solid-virtual + measureElement settles per-kind sizes  │ │
│  │  overscan: 5                                                     │ │
│  └──────────────────────────────────────────────────────────────────┘ │
│                                                                        │
│  ┌─ .agent-document-streaming-buffer (display: flex, column, gap 1px)┐│
│  │  Last STREAMING_BUFFER_SIZE = 50 nodes                            ││
│  │  Always mounted, normal flex flow, no virtualization              ││
│  │  <Index> not <For> so streaming token deltas don't remount        ││
│  └───────────────────────────────────────────────────────────────────┘│
└────────────────────────────────────────────────────────────────────────┘
```

`partitionForVirtualization(nodes)`:
- ≤ 50 nodes → everything goes to streaming buffer, virtualized region is empty.
- \> 50 nodes → trailing 50 to streaming buffer, head to virtualizer.

The split point migrates as the document grows: a node enters the streaming buffer when it's appended, then later transitions into the virtualized region as newer nodes push it past the 50-node threshold. By the time it migrates, its measurement has stabilized.

---

## 5. Why `<Index>` (not `<For>`) for the streaming buffer

`useAgentStream` produces immutable updates: each token produces a new node object (same id, fresh ref). `<For>` reconciles by **reference**, so it would unmount and remount the streaming row on every token — exactly the regression virtualization was supposed to fix. `<Index>` keys by **position** and exposes the item as a Solid signal, so `DocumentRow` stays mounted while `props.node()` re-reads on every change. (codex P1 on PR #784 caught this.)

---

## 6. Reactivity discipline (recurring failure mode)

Every block component **must read props reactively**:

- ✅ `props.node.X` at every site
- ❌ `const { node } = props` at function entry (captures once, freezes)

This bit MarkdownBlock, AgentMessageBlock, SubagentLinkBlock, ToolBlock, and the inner `Markdown` component during the Phase 2 / 3 migration. Pin toggles, status transitions (`running → success`), and token deltas all flow through prop changes without parent remounts — destructuring breaks all of them.

`DocumentNodeBody` in `DocumentRow.tsx` further uses **separate `<Show when={node.type === "X"}>` per kind** instead of `<Show when={node()}>{(n) => switch ...}`. The latter pattern uses Solid's `keyed:false` capture-once semantics and silently freezes the rendered child to whatever node arrived first — invisible until the user notices stale tool status or stuck streaming.

---

## 7. Per-kind renderer registry & estimators

`renderers.ts` registers each kind with:

| Kind | Estimator | Streaming-capable |
|---|---|---|
| `markdown` | `estimateTextHeight(content)` — 24px/line, capped at 320px | ✅ |
| `agent_message` | text-line heuristic, collapsed → 32px | ✅ |
| `user_message` | text-line heuristic, collapsed → 32px | ❌ |
| `tool` | pinned → 200px, collapsed → 32px | ❌ |
| `section` | 48px fixed | ❌ |
| `subagent_link` | 56px fixed | ❌ |

The Phase 3 perf probe (`perf-probe.ts`) compares each row's measured size against its kind's estimate at every `measureElement` call. Estimator misses > 30% surface in the dev HUD (`agent-pane-perf-section.tsx`, Ctrl+Shift+D). All probing short-circuits in production via `import.meta.env.DEV` literal folding.

---

## 8. Layout / CSS structure (the part that determines "horizontal vs vertical")

This is the meat for the bug investigation.

### 8.1 Scroll container — `.agent-document`

```scss
display: flex;
flex-direction: column;
gap: 1px;                 // ← only 1px between top-level children
contain: layout style paint;
overflow-anchor: auto;
```

Only **two** direct flex children:
1. `.agent-document-virtualizer` (the virtualized head)
2. `.agent-document-streaming-buffer` (the trailing 50)

### 8.2 Virtualized region — `.agent-document-virtualizer`

```scss
position: relative;
width: 100%;
flex-shrink: 0;
```

**No `flex-direction`.** Its children are positioned with `position: absolute; transform: translateY(start)`. Each row's vertical position comes from arithmetic in TanStack Virtual (cumulative measured/estimated sizes). The row order is correct as long as `start` is correct.

### 8.3 Streaming buffer — `.agent-document-streaming-buffer`

```scss
display: flex;
flex-direction: column;
gap: 1px;
flex-shrink: 0;
```

Each `DocumentRow` is a child div with `width: 100%`. **Vertical stacking is enforced by the flex column.**

### 8.4 Row — `.agent-document-row`

```scss
contain: layout style;
width: 100%;
```

Inside each row is a per-kind block (`<div class="agent-markdown-block">`, `<div class="agent-tool-block">`, …). The `agent-markdown-block` content is the sanitized markdown HTML produced by the `Markdown` component — `<p>`, `<table>`, `<pre>`, etc.

### 8.5 Markdown block

```scss
.agent-markdown-block {
    padding: 1px var(--space-1);
    p { margin: 1px 0; }       // ← extremely tight paragraph margin
    pre { … }
    code { … }
}
.agent-markdown-block.thinking-block {
    opacity: 0.6;
    font-style: italic;
    border-left: 2px solid var(--secondary-text-color);
    padding-left: var(--space-2);
    background: color-mix(in srgb, var(--main-text-color) 2%, transparent);
}
```

Note the `p { margin: 1px 0 }` — paragraphs inside a markdown block have effectively no separation. Combined with the `gap: 1px` between rows, **adjacent block-types end up visually adjacent with ~1-2px of breathing room**.

---

## 9. Suspects for "thinking blocks horizontal / newlines clumped around tables and thinking entries"

Three structural causes can produce this symptom. They are not mutually exclusive.

### Suspect A — Thinking deltas accumulate without paragraph breaks (parser layer)

`textToNode` and `thinkingToNode` (stream-parser.ts:157-180) append `event.content` verbatim. If the upstream model emits thinking in separate delta events without trailing newlines, the resulting `content` reads as one wall of text. The `Markdown` renderer turns it into a single `<p>` with no internal breaks — visually a horizontal run.

**Evidence to look for:** in `muxlog host '[fe]'`, search for the raw thinking deltas of an affected message and check whether they end with `\n`. If not, this is the cause.

**Fix sketch:** when a `text`/`thinking` event arrives whose previous content doesn't end in `\n`, append a `\n` between them. Alternatively, render thinking deltas as separate nodes (cut on every event) and let the row-level `gap: 1px` separate them — but that produces N tiny rows for what's logically one thought, fights the streaming buffer's accumulation, and breaks measurement.

### Suspect B — Inter-row gap is only 1px (CSS layer)

Both `.agent-document` and `.agent-document-streaming-buffer` use `gap: 1px`. Adjacent `DocumentRow`s — say, a `markdown` row containing a table followed by a `markdown.thinking-block` row — have ~1px of separation. With the `thinking-block`'s `border-left` and tinted background hugging the previous row's table border, the visual reads as "clumped" / "no newlines".

**Fix sketch:** bump `gap` to ~6-8px on the document and streaming buffer; or add per-kind margin (`.agent-markdown-block.thinking-block { margin-block: 4px; }` and `.markdown-table { margin-block: 6px; }`) so structural rows get breathing room without inflating every plain-text row.

### Suspect C — Tables share a row with surrounding markdown text (data-shape)

Because consecutive text deltas accumulate into ONE `markdown` node (§3), a single assistant turn that emits "Here's the data:" + a markdown table + "Notice the trend." produces ONE `markdown` node whose `content` contains all three. The `Markdown` component renders them as `<p>` + `<table>` + `<p>` inside one `.agent-markdown-block`. The paragraph-to-table spacing then comes from `markdown.scss`, not from `_document.scss`.

If `markdown.scss` lacks a `table { margin-block }` rule, the table butts directly against neighboring `<p>` elements. This is structurally distinct from suspect B: the symptom looks the same to the user but the CSS to fix lives in `frontend/app/element/markdown.scss`, not `_document-nodes.scss`.

**Fix sketch:** verify `markdown.scss` has `table { margin-block: var(--space-2); }` (and equivalent for `<pre>`). If not, add it. This also fixes the case where the table is followed by a `text`-then-`thinking` cut, because the table's bottom margin would still apply.

### How to discriminate between A, B, C

1. Inspect the affected DOM region with devtools and check the row count: if there are TWO `.agent-document-row` siblings (one ending in `</table>`, the next starting with `.thinking-block`), it's B. If there is ONE row containing both, it's A (thinking ran into the previous text without a node cut) or C (table inside the same markdown node as surrounding text).
2. If A or C: the row's HTML will include `<table>` followed by `<p>` (or vice-versa) inside one `.agent-markdown-block`. The fix is in `markdown.scss`.
3. If B: the rows are separate but visually touching. Fix is in `_document.scss` (gap or per-kind margin).

A 90-second screenshot inspection in the running app would resolve this — currently blocked on locating the user's screenshot.

---

## 10. Tool overlay subsystem (orthogonal but worth knowing)

Pinning a tool block triggers a `<Portal>` to `document.body` rendering an absolute-positioned `.agent-tool-content` overlay. This is necessary because `.agent-document-row`'s ancestor `.agent-document` has `contain: layout style paint`, and `content-visibility: auto` was historically applied to row wrappers; both clip the overlay to the row's bounding box. The portal escape pattern is documented in `ToolBlock.tsx` and `_tool-overlay-portal.scss`.

The overlay measures the underlying row, walks ancestors to find any CSS `zoom` (per-pane zoom, see CLAUDE.md), and applies the same zoom + `width / zoom` clamping so the visual size matches the surrounding pane. Scroll tracking is attached only while pinned (not on hover) — this avoids the one-frame reposition twitch in hover-only overlays.

---

## 11. Hover strip + bookmarks + search

`NodeHoverStrip` is a sibling of the per-kind block inside every `DocumentRow`, positioned `absolute; top: 3px; right: 6px` and faded with `opacity: 0` until the row is hovered or focus-within. It hosts bookmark, expand, "open in new pane", "open in new window", "new agent from here" actions. Visibility is pure CSS, no JS toggles.

Bookmarked nodes get `.agent-node-bookmarked`; search highlights get `.agent-node-search-match`; both are class flips in DocumentRow's `classList`.

---

## 12. Anchor-based scroll preservation (history pagination)

When `onLoadOlder` triggers (user scrolls within ~200px of top):
1. Capture the topmost visible node id + its current `start` offset (from the virtualizer or DOM offset).
2. Persist this `headAnchor` into `AgentViewState`.
3. Fetch + prepend.
4. After the next animation frame, resolve the same node id's NEW index, ask the virtualizer for its new offset (or query DOM for streaming-buffer items), and `scrollTo` so the anchor stays put.

The math uses **node ids, not pixel deltas** — robust against unsettled `ResizeObserver` heights from images/fonts loading mid-fetch. CSS `overflow-anchor: auto` on `.agent-document` handles the cases CSS can do natively (anchor element pinning when above-content reflows mid-scroll).

---

## 13. Where the layout-shift HUD looks

`startAgentLayoutShiftObserver()` (perf-probe.ts) attaches a `PerformanceObserver({ type: "layout-shift" })` and filters entries to those whose source node is inside `.agent-document`. Each shift increments per-kind counters in `agentPerfStore`. The dev HUD section polls at 1 Hz and surfaces:

- p50 / p95 / max measured size per kind
- estimator-miss rate (current vs estimated, %)
- layout-shift count attributed to `.agent-document`
- per-kind row mount count + total time

This is the right place to start when a layout symptom appears: if the bug is "thinking rows wrong height", the estimator-miss rate for `markdown` will spike. If the bug is "rows visually clumped", layout-shift count won't change but the user complains. The HUD can rule out estimator regressions in seconds.

---

## 14. Open files of record

| Path | Purpose |
|---|---|
| `frontend/app/view/agent/stream-parser.ts` | NDJSON → DocumentNode |
| `frontend/app/view/agent/types.ts` | DocumentNode discriminated union |
| `frontend/app/view/agent/virtualization/state.ts` | AgentViewState single-source-of-truth |
| `frontend/app/view/agent/virtualization/streaming-buffer.ts` | Partition logic, `STREAMING_BUFFER_SIZE = 50` |
| `frontend/app/view/agent/virtualization/renderers.ts` | Per-kind component + estimator + streaming flag |
| `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` | Hybrid renderer |
| `frontend/app/view/agent/virtualization/DocumentRow.tsx` | Single row, dispatches to per-kind block |
| `frontend/app/view/agent/virtualization/perf-probe.ts` | Dev-mode perf + estimator-miss + layout-shift |
| `frontend/app/view/agent/components/MarkdownBlock.tsx` | Markdown / thinking content |
| `frontend/app/view/agent/components/ToolBlock.tsx` | Tool summary + Portal overlay |
| `frontend/app/view/agent/components/AgentMessageBlock.tsx` | Inter-agent messages |
| `frontend/app/view/agent/styles/_document.scss` | Scroll container, virtualizer wrapper, streaming buffer, row |
| `frontend/app/view/agent/styles/_document-nodes.scss` | Per-block-kind styles (markdown, thinking, tool, …) |
| `frontend/app/element/markdown.tsx` + `markdown.scss` | Inner markdown renderer (tables, code blocks, etc.) |
| `frontend/app/devtools/agent-pane-perf-section.tsx` | Dev HUD |
| `docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md` | Authoritative design doc |

---

## 15. Recommended next step for the reported bug

Given that the symptom is "newlines around tables and thinking entries clumped":

1. **Look at the actual DOM** in the affected region (live app or screenshot) and discriminate suspect A vs B vs C using the rule in §9.
2. If B (most likely given the `gap: 1px` everywhere): bump `.agent-document` and `.agent-document-streaming-buffer` `gap` to 6-8px; optionally add `margin-block` on `.thinking-block` and `.markdown-table` for extra structural separation.
3. If C (table-inside-text-block): patch `markdown.scss` to give `table` and `pre` proper `margin-block`.
4. If A: add a newline-injection rule in `stream-parser.ts` between consecutive thinking deltas that don't end in `\n` (less invasive than re-architecting node accumulation).

Each of B and C is a 1-3 line CSS change that ships with the next bump. A is a 5-line parser change with a unit test.
