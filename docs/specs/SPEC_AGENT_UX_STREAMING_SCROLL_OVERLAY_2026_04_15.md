# SPEC: Agent Pane — Status Line, Auto-Scroll, and Tool Overlay

**Date:** 2026-04-15  
**Status:** Draft  
**Author:** AgentA

---

## Three Bugs

### Bug 1 — Status line invisible while agent is working

**Observed:** After the launch flow completes, `isLoading` is permanently `false`
(`flowRunning=false`, `agentReady=true`). Sending a message doesn't flip any
loading signal, so `AgentStatusLine` renders with `loading={false}` — the blue
pulse dot and "Working…" phrase never appear while the agent is processing.

**Root cause:**  
`isLoading = flowRunning() || !agentReady()` tracks the LAUNCH flow, not the
AGENT TURN. There is no signal for "agent is currently streaming a response."

**Desired:**  
From the moment the user presses Enter until `session_end` arrives (or the turn
errors), the status line must show `Working…` with the blue pulse dot.

**Fix — `turnActiveAtom`:**

1. Add `turnActiveAtom: SignalPair<boolean>` to `AgentAtoms` in `state.ts`, initial value `false`.
2. In `agent-view.tsx`, update `handleSendMessage` to set `turnActive = true` before sending.
3. In `useAgentStream.ts`, on `session_end` event, set `turnActive = false` (after setting session stats).
4. Also set `turnActive = false` on the `truncate` fileop (clear/reset path).
5. In `agent-view.tsx`, wire to `AgentStatusLine`:
   ```tsx
   <AgentStatusLine
       loading={status.isLoading() || agentAtoms().turnActiveAtom[0]()}
       ...
   />
   ```
6. In `handleSendMessage`, clear session stats AND set `turnActive = true`:
   ```typescript
   const handleSendMessage = (message: string): Promise<void> => {
       setSessionStats(null);
       agentAtoms().turnActiveAtom[1](true);
       return commands.sendMessage(message);
   };
   ```

---

### Bug 2 — New streamed content doesn't auto-scroll when at the bottom

**Observed:** The agent is streaming responses. The user is at the bottom of the
pane. New content appears but the view stays still — the user must manually scroll
down to see it. Only when the user has scrolled significantly upward should
auto-scroll be suppressed.

**Root cause:**  
The 50px threshold in `handleScroll` may be too tight, and there are edge cases
where `autoScroll` gets set to `false` before it should:

1. When new streaming content expands the `scrollHeight`, a browser `scroll` event
   may fire before the RAF-batched `setDocument` has fully settled, temporarily
   computing `scrollHeight - scrollTop - clientHeight > 50` and setting
   `autoScroll = false`.
2. The `content-visibility: auto` estimations on off-screen nodes may underestimate
   `scrollHeight`, causing the same false-negative.

**Desired:**  
- While `autoScroll = true`, every document update scrolls to the new bottom.
- `autoScroll` is only disabled when the user has explicitly scrolled **more than
  200px from the bottom** — a small accidental over-scroll or momentum scroll
  should not break it.
- Sending a message ALWAYS re-enables `autoScroll` and scrolls to the bottom.

**Fix:**

In `AgentDocumentView.tsx`, change:
```typescript
// Before
autoScroll = scrollHeight - scrollTop - clientHeight < 50;
// After
autoScroll = scrollHeight - scrollTop - clientHeight < 200;
```

Additionally, the auto-scroll `createEffect` currently watches `document().length`.
Because SolidJS re-runs the effect whenever `setDocument(...)` is called (signal
reference changes), this fires correctly for both appends and content updates.
However, batch flushes may coalesce multiple RAF cycles and miss intermediate
frames. Ensure the effect uses a simple accessor read:

```typescript
createEffect(() => {
    document(); // tracks any setDocument call
    logLines();
    scheduleAutoScroll();
});
```

where `scheduleAutoScroll()` replaces the inline RAF logic.

---

### Bug 3 — Tool hover expansion shifts layout / offsets visible content

**Observed:** Hovering over a one-line collapsed tool block (e.g. a Bash command)
causes surrounding content to shift ("offset everything") rather than showing
the expanded content as a pure overlay on top.

**Root cause (likely):**  
The `ToolBlock` component renders expanded content via a SolidJS `<Portal>` to
`document.body` with `position: fixed`. This is architecturally correct, but one
of the following may be failing:

1. **CSS zoom interaction:** `.agent-view--presentation` applies `style={{ zoom: zoomFactor() }}`.
   Chromium's `getBoundingClientRect()` returns zoomed-space coordinates, which
   SHOULD match `position: fixed` coordinates. But if CEF's implementation
   diverges, the portal appears at the wrong location and looks like layout shift.

2. **`content-visibility: auto` containment:** Each `.agent-document-node-wrapper`
   has `content-visibility: auto`. If a node wrapper is at the edge of the
   visible area, partial containment may interfere with the Portal's mount point
   and cause the overlay to render inline.

3. **Width calculation:** The overlay uses `width: ${r.width}px`. If the block
   has fractional pixel dimensions or the zoom multiplier creates a mismatch,
   the overlay may appear wider than the block and push the horizontal scroll.

**Desired:**  
Hovering a collapsed tool block must:
- Show the expanded content as a floating overlay (`position: fixed`) directly
  below (or above) the summary row.
- Never cause any sibling node to shift position.
- Never cause the scroll container to change `scrollHeight`.
- Dismiss instantly on mouse-leave (current 0ms timeout is correct).

**Fix:**

1. **Verify the portal is actually rendering to body:** in `ToolBlock.tsx`, add a
   `console.debug` when `hovered = true` to confirm `overlayRect` is set correctly
   and the portal element is attached to `document.body`, not to the block.

2. **Compensate for CSS zoom in position calculation:**  
   If `zoom !== 1`, the `getBoundingClientRect()` coords returned by CEF are in
   the unzoomed space. Multiply by zoom before applying `position: fixed`:
   ```typescript
   const zoom = parseFloat(
       getComputedStyle(document.documentElement).zoom || "1"
   );
   // OR walk up to find the zoomed ancestor and read its zoom
   ```
   
   Actually, Chrome/CEF `getBoundingClientRect()` already returns VIEWPORT pixels
   (accounting for zoom), so no correction should be needed. If it IS needed, wrap
   in `agentViewZoom()` accessor passed down from `AgentPresentationView`.

3. **Ensure the overlay width matches the block:**  
   Instead of `width: ${r.width}px`, use `min-width: ${r.width}px` so content
   wider than the block can grow rightward without causing horizontal scroll in
   the document.

---

## Summary Table

| Issue | Signal | Fix location | Risk |
|-------|--------|--------------|------|
| Status line invisible | Missing `turnActiveAtom` | state.ts, agent-view.tsx, useAgentStream.ts | Low |
| No auto-scroll at bottom | Threshold 50px too small | AgentDocumentView.tsx line ~182 | Low |
| Overlay shifts content | Portal position / CSS zoom | ToolBlock.tsx + agent-view.scss | Medium |

---

## Implementation Order

1. **Bug 1** — `turnActiveAtom` (state.ts + agent-view.tsx + useAgentStream.ts)
2. **Bug 2** — Increase threshold to 200px (one-liner in AgentDocumentView.tsx)
3. **Bug 3** — Investigate portal overlay, apply zoom fix or min-width

All three can ship in one PR.
