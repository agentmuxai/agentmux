# SPEC: Tool Block UX Polish — Hover Delay, Collapse Animation, Post-Completion Hold, Thinking Label, Scroll Isolation

**Date:** 2026-05-23  
**Status:** Ready for implementation  
**Files touched:** `ToolBlock.tsx`, `ToolOverlayLog.tsx`, `_document-nodes.scss`, `_tool-overlay-portal.scss`

---

## Background

When the portal overlay was replaced with the inline panel
(`SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md`, PR #888), three UX
regressions were introduced alongside one label that never got updated:

1. The **150 ms hover enter delay** was removed (comment at `ToolBlock.tsx:65–72`
   explains the rationale — inline panel removes "dead space" — but the delay
   also prevents accidental expansions while scrolling past tools).
2. The **panel collapse is instant** — no exit animation, making the pane feel
   abrupt when a running tool finishes and the block snaps shut.
3. The **post-completion collapse is also instant** — the panel disappears the
   moment `status` flips from `running` → `success`, giving the user no time
   to read the final output.
4. The **"Running..." placeholder text** was never updated to reflect that Claude
   is thinking, not executing a shell command.
5. The **log panel has no scroll isolation** — when the user scrolls to the top
   or bottom of the log, the wheel event propagates to the agent pane and scrolls
   the entire conversation, even though the cursor is still over the log.

This spec addresses all five.

---

## Change 1 — Restore the 150 ms hover enter delay

### Why
Scrolling through a long agent response causes the cursor to briefly enter
tool-block rows. Without a debounce, every such row expands and collapses in
rapid succession, creating visual noise. The 150 ms gate prevents this.
Leave is instant (no grace window needed — the inline panel is inside the
same `.agent-tool-block` bounding box, so `mouseleave` only fires when the
cursor genuinely exits the whole block).

### Implementation — `frontend/app/view/agent/components/ToolBlock.tsx`

```tsx
// Before (lines 76–78):
const [hovering, setHovering] = createSignal(false);
const handleMouseEnter = () => setHovering(true);
const handleMouseLeave = () => setHovering(false);

// After:
const HOVER_ENTER_DELAY_MS = 150;

const [hovering, setHovering] = createSignal(false);
let enterTimer: ReturnType<typeof setTimeout> | undefined;

const handleMouseEnter = () => {
    enterTimer = setTimeout(() => setHovering(true), HOVER_ENTER_DELAY_MS);
};
const handleMouseLeave = () => {
    clearTimeout(enterTimer);
    setHovering(false);
};
onCleanup(() => clearTimeout(enterTimer));
```

Also add `onCleanup` to the import line:
```tsx
// Before:
import { Show, createSignal, type JSX } from "solid-js";

// After:
import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
```

---

## Change 2 — Animate collapse, instant open

### Why
The panel should feel like it's sliding shut (200 ms ease-out) so the user
can track what happened, but it should open **instantly** (after the hover
delay) so there is no sluggishness when intentionally expanding.

### Approach
Replace the `<Show when={expanded()}>` gate with an always-rendered div whose
visibility is CSS-driven. This avoids a SolidJS exit-animation workaround
(SolidJS has no built-in leave transition) and keeps the implementation simple.

The panel is hidden via `max-height: 0` + `overflow: hidden` when the block
has the `--hidden` modifier class. The transition fires on collapse only.

### Implementation — `frontend/app/view/agent/components/ToolBlock.tsx`

```tsx
// Before:
<Show when={expanded()}>
    <div class="agent-tool-panel" ...>
        <ToolBlockOverlay ... />
    </div>
</Show>

// After — always rendered, visibility via CSS modifier:
<div
    class={clsx("agent-tool-panel", { "agent-tool-panel--hidden": !expanded() })}
    onClick={(e) => e.stopPropagation()}
    onMouseEnter={handleMouseEnter}
    onMouseLeave={handleMouseLeave}
>
    <ToolBlockOverlay ... />
</div>
```

### Implementation — `frontend/app/view/agent/styles/_document-nodes.scss`

```scss
.agent-tool-panel {
    // ... existing properties ...
    overflow: hidden; // required for max-height collapse to clip content

    // No transition here — open is instant (after the hover delay).
    // The --hidden modifier carries the collapse transition so it only
    // fires on the way out.
    &--hidden {
        max-height: 0 !important;
        padding-top: 0 !important;
        padding-bottom: 0 !important;
        margin-top: 0 !important;
        margin-bottom: 0 !important;
        opacity: 0;
        transition: max-height 200ms ease-out, opacity 150ms ease-out,
                    padding 200ms ease-out, margin 200ms ease-out;
    }
}
```

**Why "instant open" works:** The `transition` property only exists on the
`&--hidden` modifier. When the class is removed (expand), the browser sees no
transition rule and snaps open immediately. When the class is added (collapse),
the browser picks up the transition from the new rule and animates.

---

## Change 3 — 1-second post-completion hold before auto-collapse

### Why
`autoExpanded()` currently returns `false` the instant `status` changes from
`running` → `success`. The panel snaps shut with zero reading time. A 1-second
grace period lets the user see the final output line before the block collapses.

This only affects the **auto-expand** path (running tools). A user who has
manually pinned the block is unaffected — `props.pinned` still wins.

### Implementation — `frontend/app/view/agent/components/ToolBlock.tsx`

```tsx
// New signal — stays true for 1s after a running tool completes:
const [postCompletionHold, setPostCompletionHold] = createSignal(false);
createEffect(() => {
    const s = props.node.status;
    if (s !== "running" && s !== "pending_approval" && s !== "failed") {
        if (postCompletionHold()) return;
        setPostCompletionHold(true);
        const t = setTimeout(() => setPostCompletionHold(false), 1000);
        onCleanup(() => clearTimeout(t));
    }
});

// Updated autoExpanded:
const autoExpanded = (): boolean => {
    const s = props.node.status;
    return s === "running" || s === "pending_approval" || s === "failed"
        || postCompletionHold();
};
```

---

## Change 4 — "Running..." → "Thinking..."

### Why
The tool blocks represent Claude's reasoning steps. "Thinking..." better
matches the mental model than "Running...", which implies a shell process.

### Implementation — `frontend/app/view/agent/components/ToolOverlayLog.tsx`

```tsx
// Before (line 177):
<span class="agent-tool-spinner">⏳</span> Running...

// After:
<span class="agent-tool-spinner">⏳</span> Thinking...
```

---

---

## Change 5 — Scroll isolation on the log panel

### Why
Browsers implement **scroll chaining** by default: when a scrollable element
hits its top or bottom boundary, the remaining wheel delta propagates to the
nearest scrollable ancestor. For the tool log panel this means scrolling to
the end of a long output then continuing to scroll moves the entire agent
conversation — the user loses their place in the transcript while their cursor
is still over the log. It's unexpected and disorienting.

### Fix
`overscroll-behavior: contain` on `.agent-tool-overlay-log` tells the browser
to absorb the wheel event at this element's boundary and not chain it upward.
One CSS property, no JS.

### Implementation — `frontend/app/view/agent/styles/_tool-overlay-portal.scss`

```scss
// Before (line 45):
.agent-tool-overlay-log {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-1-5) 10px var(--space-1-5) var(--space-2);
    ...
}

// After — add one line:
.agent-tool-overlay-log {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain; // prevent scroll chaining to agent pane
    padding: var(--space-1-5) 10px var(--space-1-5) var(--space-2);
    ...
}
```

### Browser support
`overscroll-behavior` is supported in all Chromium-based browsers (Electron
included) since Chrome 63. No fallback needed for the Tauri/Electron target.

---

## Summary of file changes

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/ToolBlock.tsx` | Re-add 150ms hover enter delay; post-completion hold signal + effect; replace `<Show>` panel with always-rendered + CSS modifier class |
| `frontend/app/view/agent/components/ToolOverlayLog.tsx` | `"Running..."` → `"Thinking..."` |
| `frontend/app/view/agent/styles/_document-nodes.scss` | `overflow: hidden` on panel; `&--hidden` modifier with 200ms collapse transition |
| `frontend/app/view/agent/styles/_tool-overlay-portal.scss` | `overscroll-behavior: contain` on `.agent-tool-overlay-log` |

---

## Behaviour matrix after this spec

| Scenario | Before | After |
|----------|--------|-------|
| Cursor skims over a collapsed tool | Instant expand | 150ms delay — no expand on quick pass |
| Cursor enters and stays | Instant expand | Expands after 150ms |
| Cursor leaves expanded block | Instant collapse, no animation | 200ms slide-out animation |
| Running tool finishes | Panel snaps shut immediately | Panel stays open 1s then collapses with animation |
| No chunks yet, tool running | "⏳ Running..." | "⏳ Thinking..." |
| Scroll reaches end of log | Chains to agent pane scroll | Stops at log boundary |
| User has pinned block | Already works | Unaffected — pin still wins |
