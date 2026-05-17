# SPEC: Tool Auto-Expand Panel (Replace Portal Overlay)

**Status:** Draft
**Date:** 2026-05-16
**Author:** AgentA
**Replaces:** `SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md` §3.4 (portal overlay), `SPEC_UNIFIED_TOOL_HOVER_OVERLAY_2026_05_13.md` (whole spec — hover overlay deprecated)
**Related:** `SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md` (transport layer — already working)

---

## 1. Problem

Today's tool block has a **portal-based hover/click overlay** that holds the live log + rich result. While the user is watching a tool run, the overlay only renders when the user actively hovers or pins it. In practice:

- Most users don't hover/pin during a 1–5 second tool run; they just watch the chat.
- The overlay's lifecycle is hover-driven, so it mounts and unmounts repeatedly during the same tool's run (we see this in trace: ToolOverlayLog's component body re-runs every 3–4s as the user's mouse drifts in and out of the row).
- Critically, **during the actual streaming window** (when bash chunks arrive), the overlay is typically unmounted — so the user perceives "Running… then everything dumps at the end" even though chunks ARE landing in the reducer in real time.
- The Portal escapes the document's paint containment but in exchange the lifecycle of the overlay is decoupled from the tool's lifecycle.

The result: PTY streaming, DSR responder, ANSI strip, reducer state, and reactive chain are all working end-to-end (verified in 2026-05-16's diag run — chunks reach the reducer 1.05 s apart for `stream-test.sh`), but the user sees none of it because the overlay isn't open when chunks arrive.

## 2. Goal

When a tool is actively running, its output is **visible by default**, in normal document flow, without the user needing to hover, click, or pin. When the tool completes, the panel auto-collapses so the conversation stays readable. The user can manually toggle (override) at any time.

```
┌─ chat ──────────────────────────────────────────────────┐
│ User: run stream-test.sh                                │
│ Maks: I'll run that for you now                         │
│                                                         │
│ ⏳ Bash(./stream-test.sh)                               │ ← summary row (always visible)
│ ┌─────────────────────────────────────────────────────┐ │
│ │ === stream-test starting at 17:23:18.123 ===        │ │ ← auto-expanded WHILE running
│ │ step 1  17:23:19.180                                │ │   (panel inline in doc flow)
│ │ step 2  17:23:20.245                                │ │
│ │ step 3  17:23:21.298                                │ │
│ │ step 4  17:23:22.... ▌                              │ │
│ └─────────────────────────────────────────────────────┘ │
│                                                         │
│ ... (then when bash completes:)                         │
│                                                         │
│ ✓ Bash(./stream-test.sh)  (5.1s) ▸                      │ ← summary row, panel auto-collapsed
│                                                         │
│ Maks: done. Output was as expected.                     │
└─────────────────────────────────────────────────────────┘
```

User can click the row to re-expand and see the final result (current `ToolOverlayResult` rendering — `BashOutputViewer`, `DiffViewer`, etc.).

## 3. Non-goals

- **Not changing the streaming backend.** The PTY rework + DSR + ANSI strip already land chunks correctly. This spec is purely a UI restructure.
- **Not changing the data model.** `ToolNode.log` + `ToolNode.result` stay as-is from `SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md` §3.1.
- **Not adding animation timers or transitions.** Per `feedback_no_timers_or_delays.md`, expansion/collapse is instant. The collapsed↔expanded transition happens at the same moment as the status flip.
- **Not removing pin semantics entirely** — pin becomes a "user override" against the auto-managed state (see §6.4).
- **Not redesigning the action bar.** Open-in-pane, bookmark, etc. stay where they are (inside the panel content, same component).

## 4. Design

### 4.1 Replace the portal with an inline panel

Today's `ToolBlock.tsx`:

```jsx
<div class="agent-tool-block">
    <div class="agent-tool-summary" onClick={onTogglePin}>...</div>
    <Show when={expanded()}>                       {/* hover OR pin */}
        <Portal>
            <div ... style={overlayStyle()}>       {/* positioned absolutely */}
                <ToolBlockOverlay node={props.node} ... />
            </div>
        </Portal>
    </Show>
</div>
```

New shape:

```jsx
<div class="agent-tool-block">
    <div class="agent-tool-summary" onClick={onToggleExpanded}>...</div>
    <Show when={isExpanded()}>                     {/* auto-while-running OR user-override */}
        <div class="agent-tool-panel">             {/* inline, normal flow */}
            <ToolBlockOverlay node={props.node} ... />
        </div>
    </Show>
</div>
```

The `<Portal>` goes away. `ToolBlockOverlay`'s existing three-slot layout (header / log / actions) renders inside `.agent-tool-panel` and inherits the document flow's width.

### 4.2 Auto-expansion logic

Replace `expanded()` (which was `pinned || hovering()`) with `isExpanded()`:

```ts
const isExpanded = createMemo(() => {
    const explicit = userExpandState(props.node.id);   // see §4.3
    if (explicit !== "auto") return explicit === "open";
    // Auto: open while running, closed otherwise.
    return props.node.status === "running" || props.node.status === "pending_approval";
});
```

Truth table:

| `status`            | `userExpandState` = `"auto"` | `"open"` | `"closed"` |
|---------------------|------------------------------|----------|------------|
| `running`           | **expanded**                 | expanded | collapsed  |
| `pending_approval`  | **expanded**                 | expanded | collapsed  |
| `success`           | collapsed                    | expanded | collapsed  |
| `failed`            | **expanded** (see §4.5)      | expanded | collapsed  |
| `denied`            | collapsed                    | expanded | collapsed  |

When status flips from `running` → `success`, `isExpanded()` re-evaluates and the panel collapses. No timers — the collapse is driven by the same signal that flips status.

### 4.3 User override state

Replace today's `pinnedNodes: Set<string>` with `userExpandState: Map<string, "open" | "closed">`. Absence from the map means `"auto"` (default).

```ts
interface DocumentState {
    // existing fields...
    pinnedNodes: Set<string>;                  // DEPRECATED — migrate to userExpandState
    userExpandState: Map<string, "open" | "closed">;
}
```

Click on the tool summary row cycles through three states based on current visual state:

| User clicks while | Action |
|---|---|
| Auto-expanded (running) | Set `"closed"` — user wants it out of the way |
| Auto-collapsed (completed) | Set `"open"` — user wants to see the result |
| Manually `"open"` | Set `"closed"` |
| Manually `"closed"` | Remove from map (back to auto) |

In practice the user gets the obvious behavior: clicking always inverts the current visual state, and a click-then-status-change goes back to auto-managed.

### 4.4 Failed tools stay expanded

`failed` status defaults to expanded under auto policy (last row of the table in §4.2). Rationale: same as `tool-collapse.md` §"Exceptions" — error output is the first thing a user wants to see. The user can manually collapse with one click.

### 4.5 Streaming-still-open edge case

`ToolNode.log.open` can stay `true` for a brief window after `status` flips to `success` — chunks in the publisher queue that haven't been HTTP-published yet. The auto-policy keys off `status`, not `log.open`, so the panel collapses immediately on status flip. Any chunks that arrive post-collapse still land in `log.chunks` (the reducer keeps appending); they'll be visible when the user manually re-expands the panel. This is the same behavior as today's overlay; no regression.

### 4.6 Removed components / paths

- `Portal` import from `ToolBlock.tsx` — gone.
- `overlayRect`, `overlayUp`, `overlayStyle` signals — gone (no positioning math needed in flow).
- `getAncestorZoom`, `findScrollParent`, `OVERLAY_MAX_HEIGHT_PX`, scroll-tracking effect, resize-listener effect — all gone.
- `hovering` signal + `enterTimer` / `leaveTimer` + `handleMouseEnter` / `handleMouseLeave` + `HOVER_ENTER_DELAY_MS` / `HOVER_LEAVE_DELAY_MS` — all gone. Hover is no longer a trigger for expansion.
- `NodeHoverStrip` for tool rows stays suppressed (per `SPEC_UNIFIED_TOOL_HOVER_OVERLAY_2026_05_13.md` §3.1), but the hover detection logic itself can also be retired here.

### 4.7 Layout / CSS

`.agent-tool-panel` lives **inside** `.agent-tool-block`, between the summary row and the next document node. It uses normal flex layout, no absolute positioning.

```scss
.agent-tool-block {
    .agent-tool-summary { ... }    // unchanged

    .agent-tool-panel {
        margin: 4px 0 8px 24px;    // indented under the summary row
        border-left: 2px solid var(--accent-color-muted, #2bd4a833);
        padding: 6px 8px;
        background: var(--surface-elevated, rgba(255, 255, 255, 0.02));
        border-radius: 0 4px 4px 0;
        max-height: 50vh;          // bound vertical growth; long output scrolls
        overflow-y: auto;
        font-family: var(--fixed-font, monospace);
        font-size: 12px;
    }
}
```

Auto-scroll to bottom while streaming: the existing `ToolOverlayLog` already has a `stickToBottom` effect via `scrollRef.scrollTop = scrollRef.scrollHeight`. It keeps working — the scroll container is now the inline panel instead of the portal'd div.

### 4.8 Content-visibility / paint containment

The reason today's overlay is portal'd is that `.agent-document-node-wrapper` has `content-visibility: auto`, which clips children. With the panel inline (a child of `.agent-tool-block`, which is itself inside `.agent-document-node-wrapper`), it's contained.

Two options to handle this:

- **A.** Remove `content-visibility: auto` from the wrapper for tool nodes specifically. Tool nodes are heterogeneous in height when expanded; we lose some virtualization perf but the existing virtualizer + estimator already handles variable-height rows.
- **B.** Leave `content-visibility: auto` and rely on the virtualizer's `measureElement` to re-measure tool nodes when they expand/collapse. The measurement is reactive on every height change via `data-index` attribute.

Recommend **B**: virtualizer already supports variable-height children; tool node measurement should update on panel expand/collapse just like any other row size change. If perf degrades, fall back to A.

## 5. Reducer / state changes

### 5.1 `DocumentState`

```ts
interface DocumentState {
    collapsedNodes: Set<string>;            // unchanged — for non-tool collapsibles
    pinnedNodes: Set<string>;               // DEPRECATED — keep for one release for migration
    userExpandState: Map<string, "open" | "closed">;   // NEW
    scrollPosition: number;
    selectedNode: string | null;
    filter: { /* ... */ };
}
```

### 5.2 Migration

On document state restore (from snapshot per `SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md`), any entry in `pinnedNodes` becomes `userExpandState.set(id, "open")`. After one release cycle, `pinnedNodes` and its setter (`onTogglePin`) are removed.

### 5.3 Click handler

`onClick` on the summary row dispatches a `ToggleToolExpansion` command:

```ts
type ToggleToolExpansion = { type: "ToggleToolExpansion"; toolId: string };
```

Reducer logic (in `agent-pane-state-store.ts`):

```ts
case "ToggleToolExpansion": {
    const id = command.toolId;
    const current = state.userExpandState.get(id);
    const tool = findTool(state, id);
    if (!tool) return { state, events: [] };
    const isAutoExpanded = tool.status === "running"
                        || tool.status === "pending_approval"
                        || tool.status === "failed";
    const visuallyExpanded =
        current === "open" ? true :
        current === "closed" ? false :
        isAutoExpanded;
    const next = new Map(state.userExpandState);
    if (visuallyExpanded) {
        // Click while expanded → collapse
        if (current === "open") {
            next.delete(id);                   // open → auto (may stay open if running)
        } else {
            next.set(id, "closed");            // auto → closed
        }
    } else {
        // Click while collapsed → expand
        if (current === "closed") {
            next.delete(id);                   // closed → auto (may stay closed if completed)
        } else {
            next.set(id, "open");              // auto → open
        }
    }
    return {
        state: { ...state, userExpandState: next },
        events: [{ type: "tool-expansion-toggled", toolId: id, to: next.get(id) ?? "auto" }],
    };
}
```

## 6. Open questions

- **Q1.** Should we animate the height change at all? Currently the spec says no (instant collapse on status flip, per `feedback_no_timers_or_delays.md`). But the snap might feel abrupt for tools that complete quickly. Consider: collapse only after the user's mouse hasn't been over the row for some condition that's NOT a timer? Open for discussion.
- **Q2.** Multiple tools running in parallel (Claude's parallel tool_use blocks). Each panel auto-expands; the chat can become tall. Acceptable, or should we cap concurrent expansions to N?
- **Q3.** Should the live-tail inline in the collapsed row (the recent shipped attempt) stay even after this spec lands? It's redundant for the auto-expand-while-running case but useful for tools the user manually collapsed mid-run. Lean: keep it.
- **Q4.** Should `pending_approval` (e.g. dangerous Bash awaiting user confirmation) be visually distinct from `running`? Today they share the spinner. With auto-expand, both expand. Probably want a different border-left color for `pending_approval`.

## 7. Plan

### Phase A — replace portal with inline panel, no behavior change

1. In `ToolBlock.tsx`: drop the `<Portal>` and the positioning math. Render `<ToolBlockOverlay>` inside `.agent-tool-panel` directly. Keep `expanded()` = `pinned || hovering()` for now.
2. CSS: new `.agent-tool-panel` styles (§4.7).
3. Verify: hover/pin still works, just renders inline. Same UX, simpler code.

Acceptance: tool overlays render inline; no portal in the DOM tree for `.agent-tool-content--portal` selector.

### Phase B — auto-expand while running

1. Add `userExpandState: Map<string, "open" | "closed">` to `DocumentState` + migration from `pinnedNodes`.
2. Replace `expanded()` with `isExpanded()` per §4.2.
3. Wire `onClick` to dispatch `ToggleToolExpansion` per §5.3.
4. Strip the hover signals + timers from `ToolBlock.tsx` (§4.6 list).

Acceptance: running a `for…sleep…echo` loop shows the chunk panel expanded automatically; the panel collapses on success without user interaction; click on the summary row toggles correctly through the auto/open/closed cycle.

### Phase C — cleanup

1. Strip the live-log-diag instrumentation (gate-eval / partition-eval / docSignal-fire / live-tail check) once Phase B is verified.
2. Decide on Q3 (live-tail in collapsed row): if keep, the user-collapsed-while-running case still has visibility.
3. Decide on Q4 (`pending_approval` border color).

Acceptance: clean log output during a tool run; the live-tail and the auto-expand panel both reinforce the "you can see what's happening" UX.

## 8. Test plan

### 8.1 Visual

Run `stream-test.sh` via Maks. Expectations:

- The tool summary row appears immediately when Claude starts streaming the tool_use.
- The panel below the summary row auto-expands once the tool transitions to `running` (or stays expanded from the moment the tool node is added, since the initial status is `running`).
- As bash emits each `step N` line, the line appears in the panel within a couple frames of the chunk landing in the reducer.
- Five `step N` lines appear at 1-second intervals.
- When bash exits, the panel auto-collapses.
- Click on `✓ Bash(./stream-test.sh)` re-expands the panel; the full output is visible.

### 8.2 Edge cases

- **Multiple parallel tools.** Ask Claude to run 3 parallel reads + a bash. Each tool's panel auto-manages independently.
- **User collapses mid-run.** Click the row while chunks are streaming → panel collapses → keep streaming chunks land in `log.chunks` but UI doesn't display them → click to re-expand → user sees all accumulated chunks. (No chunks lost — they're in the reducer the whole time.)
- **Failed tool.** Run `false`. Status flips to `failed`, panel stays expanded.
- **Persisted state restore.** Open agent pane, run tool, close pane, reopen pane. Tool from snapshot: status=success → auto-collapsed. Click → expands → shows persisted output.

### 8.3 Reducer

Unit tests for `ToggleToolExpansion`:

- `auto` + status=`running` → click → `"closed"`
- `"closed"` + status=`running` → click → `delete` (back to `"auto"`)
- `auto` + status=`success` → click → `"open"`
- `"open"` + status=`success` → click → `delete`
- Click on non-existent toolId → no-op (no event emitted)

## 9. References

### Code

- `frontend/app/view/agent/components/ToolBlock.tsx` — current Portal-based overlay, lines 257–322
- `frontend/app/view/agent/components/ToolBlockOverlay.tsx` — content unchanged in this spec
- `frontend/app/view/agent/components/ToolOverlayLog.tsx` — keep as-is (current scroll-to-bottom effect at line 122 still works inside inline panel)
- `frontend/app/view/agent/state.ts` — `DocumentState` shape, `pinnedNodes` lines ~94
- `frontend/app/store/agent-pane-state-store.ts` — where `ToggleToolExpansion` reducer lives
- `frontend/app/view/agent/styles/_document-nodes.scss` — `.agent-tool-summary`, `.agent-tool-content--portal` rules (the portal rules can go)

### Prior specs

- [`tool-collapse.md`](./tool-collapse.md) — original hover-expand spec (predecessor; this spec changes the trigger from hover to auto-managed)
- [`SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md`](./SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md) — log streaming data model (unchanged here)
- [`SPEC_UNIFIED_TOOL_HOVER_OVERLAY_2026_05_13.md`](./SPEC_UNIFIED_TOOL_HOVER_OVERLAY_2026_05_13.md) — deprecated; the hover overlay this consolidates is itself being removed
- [`SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md`](./SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md) — paint-containment context for why the Portal exists
- [`SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md`](./SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md) — transport-layer fix; already shipped to bashwrap

### Reference behavior

- **VS Code agent mode** (microsoft/vscode `runInTerminalTool.ts`) — keeps the tool collapsed by default; click-to-expand reveals a **live xterm.js view**. Different UX choice — they show full terminal in a popout; we render a chunk list inline. The shared insight: streaming visibility must be tied to tool lifecycle, not user interaction.
- **`feedback_no_timers_or_delays.md`** — no fade-in / settle timer on auto-collapse. The transition fires on the same reactive signal that flips status.
- **`feedback_solidjs_reactive_leak.md`** — keep reactive accesses to `props.node` inline at JSX expression sites; no destructuring; no `createMemo` of single prop values.
