# Spec: Tool Hover Overlay + Scroll-on-Type

**Date:** 2026-04-13
**Status:** Draft — ready to implement
**Scope:** Two related agent-pane UX fixes that both prevent document layout from shifting under the user.

---

## 1. Problem statement

Two bugs in 0.33.108 both come from the same underlying principle: *things appearing or changing in the agent pane should not push the document around*.

### 1.1 Tool hover-expand pushes content down

When you hover a collapsed tool block, the `<Show when={expanded()}>` mounts `.agent-tool-content` as a flow child of `.agent-tool-block`. The block grows vertically in document flow and shoves every tool, message, and bookmark below it down the page. Moving the mouse to follow the shifted content triggers mouseenter on a *different* block, which also expands, cascading the shift.

Symptom: "hover a tool and watch the rest of the document slide down; try to follow and it keeps sliding." Unusable as a browsing experience.

### 1.2 Typing in the composer doesn't scroll the document to latest

When the user has scrolled up in a long session to read something, `autoScroll = false` in `AgentDocumentView` (disabled by the scroll-listener when `scrollHeight - scrollTop - clientHeight >= 50`). The user then starts typing a follow-up message, but the document stays wherever they scrolled to — so their composer input appears at the bottom of the viewport while the latest agent content is offscreen above. There's no visual anchor between "what I'm typing" and "what I'm responding to."

Intuitive expectation: the moment I start typing a reply, the document should scroll to the latest content so I can see what my message is about to sit under.

### 1.3 Shared principle

Both bugs are "UI affordances shouldn't move the document." The tool overlay fix keeps the document in place when a tool expands. The scroll-on-type fix moves the document (once, to the bottom) at the user's implicit request (starting to type). Together they make the document a stable surface that the user controls.

---

## 2. Fix 1 — Tool hover-expand overlays instead of pushing

### 2.1 Current structure

```tsx
// frontend/app/view/agent/components/ToolBlock.tsx
<div class="agent-tool-block" onMouseEnter={…} onMouseLeave={…}>
    <div class="agent-tool-summary">
        <span class="agent-tool-status-icon">{icon}</span>
        <span class="agent-tool-name">{name}</span>
        {duration}
        <span class="agent-tool-ellipsis">…</span>
    </div>
    <Show when={expanded()}>
        <div class="agent-tool-content">{renderToolContent()}</div>
    </Show>
</div>
```

`.agent-tool-content` is a flow child — it takes document space when mounted.

### 2.2 Target structure

`.agent-tool-content` becomes an **absolutely-positioned overlay** anchored to `.agent-tool-block`. The block itself keeps its one-line height in the flow. The content floats down over whatever is below.

### 2.3 CSS changes — `frontend/app/view/agent/agent-view.scss`

```scss
.agent-tool-block {
    // NEW: anchor for the absolute content overlay
    position: relative;

    // … existing border-left, cursor, hover, status variant classes …

    .agent-tool-content {
        // REPLACE existing styles with:
        position: absolute;
        top: 100%;               // directly below the summary row
        left: 0;
        right: 0;
        z-index: 20;             // over sibling tool blocks
        background: var(--main-bg-color);
        border: 1px solid var(--border-color);
        border-top: none;        // seamless with the summary row above
        border-radius: 0 0 3px 3px;
        box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
        max-height: min(400px, 60vh);
        overflow-y: auto;
        // Keep existing text-selection + cursor behavior
        padding: 6px 10px 8px 20px;
        cursor: text;
        user-select: text;
    }

    // JS toggles this class when the block is near the bottom of the
    // scroll container, flipping the overlay to appear above the
    // summary row instead of below.
    &.overlay-up .agent-tool-content {
        top: auto;
        bottom: 100%;
        border-top: 1px solid var(--border-color);
        border-bottom: none;
        border-radius: 3px 3px 0 0;
        box-shadow: 0 -6px 20px rgba(0, 0, 0, 0.35);
    }
}
```

### 2.4 JS changes — `frontend/app/view/agent/components/ToolBlock.tsx`

Add an `overlayUp` signal. On `mouseenter`, measure the block's position relative to its scrollable ancestor and decide whether to flip the overlay up.

```ts
const [hovered, setHovered] = createSignal(false);
const [overlayUp, setOverlayUp] = createSignal(false);

// Find the nearest scrollable ancestor. Walks once per mouseenter —
// not per frame, not per keystroke, so the layout read is amortized.
function findScrollParent(el: HTMLElement): HTMLElement | null {
    let parent: HTMLElement | null = el.parentElement;
    while (parent && parent !== document.body) {
        const style = getComputedStyle(parent);
        if (style.overflowY === "auto" || style.overflowY === "scroll") {
            return parent;
        }
        parent = parent.parentElement;
    }
    return null;
}

const handleMouseEnter = (e: MouseEvent) => {
    setHovered(true);
    // Decide whether the overlay has enough room below. We don't know
    // the exact expanded height yet (content is still collapsed), so
    // use the CSS max-height as a worst case and flip if there's
    // less than that much room below the block.
    const block = e.currentTarget as HTMLElement;
    const blockRect = block.getBoundingClientRect();
    const scrollParent = findScrollParent(block);
    const parentBottom = scrollParent
        ? scrollParent.getBoundingClientRect().bottom
        : window.innerHeight;
    const spaceBelow = parentBottom - blockRect.bottom;
    // 400px matches the CSS max-height cap. Under that, flip upward.
    setOverlayUp(spaceBelow < 400);
};

const handleMouseLeave = () => {
    setHovered(false);
    // overlayUp stays — it gets recomputed on next mouseenter
};
```

Wire them into the existing `onMouseEnter` / `onMouseLeave` on `.agent-tool-block`, and add `"overlay-up": overlayUp()` to the clsx class list.

### 2.5 Edge cases

**Mouse path from summary to content.**
Because the absolute overlay is still a **DOM descendant** of `.agent-tool-block`, `mouseleave` fires on the outer block only when the cursor leaves both the summary row and the overlay combined. The user can move freely from the collapsed row down into the expanded content without the overlay closing. No delay needed.

**Pinned tools.**
Click-to-pin should keep the overlay behavior: the content stays as a floating panel. Pinned is a visual state (border accent, maybe bolder shadow), not a "take inline space" state. This keeps layout stable regardless of whether 0, 1, or 10 tools are pinned.

**Force-expanded states (running/failed).**
A failed tool that has been "always expanded" takes flow space in the current design. With this spec, running/failed also become overlays. Argument for: consistency, document never shifts from any state change. Argument against: you can't scroll past a failed tool to see what's underneath without closing it. **Proposed resolution:** running/failed tools keep their force-expanded behavior BUT as inline content rather than overlay — so they retain existing behavior. Hover-triggered expansion is the overlay case. This makes the overlay specifically a "transient" affordance.

Practical split:

| State | Render mode | Reason |
|---|---|---|
| Collapsed | 1-line summary, nothing else | Default |
| Hovered (transient) | Summary inline + content **overlay** | Don't shift the doc on hover |
| Pinned (sticky user action) | Summary inline + content **overlay** | Don't shift the doc on click either |
| Running (persistent state) | Summary + content **inline** (takes flow space) | User wants to watch progress; scrolling past it is fine |
| Failed (persistent state) | Summary + content **inline** (takes flow space) | Errors must be visible without hover; user should be able to scroll past them |

Implementation: the overlay class (`position: absolute`) applies only when `hovered() || pinned`. When `running || failed`, the content renders inline as today. Two separate render paths in the JSX:

```tsx
return (
    <div class={clsx("agent-tool-block", {
        collapsed: !expanded(),
        expanded: expanded(),
        pinned: props.pinned,
        "overlay-up": overlayUp(),
        "overlay-mode": expanded() && !forceExpanded(),
        running: props.node.status === "running",
        success: props.node.status === "success",
        failed: props.node.status === "failed",
    })} onMouseEnter={handleMouseEnter} onMouseLeave={handleMouseLeave}>
        <div class="agent-tool-summary" onClick={props.onTogglePin}>
            {/* … */}
        </div>
        <Show when={expanded()}>
            <div class="agent-tool-content" onClick={(e) => e.stopPropagation()}>
                {renderToolContent()}
            </div>
        </Show>
    </div>
);
```

And in SCSS:

```scss
.agent-tool-block.overlay-mode .agent-tool-content {
    position: absolute;
    top: 100%;
    // … (overlay styles above)
}
// If NOT .overlay-mode, .agent-tool-content keeps its current inline styles.
```

---

## 3. Fix 2 — Typing in composer scrolls document to latest

### 3.1 Current behavior

- `AgentDocumentView` maintains a local `autoScroll` boolean. When `true`, every stream flush sets `scrollRef.scrollTop = scrollRef.scrollHeight`.
- `autoScroll` is set to `false` by:
  - The scroll-position listener if the user scrolls up (`scrollHeight - scrollTop - clientHeight >= 50`)
  - A `scrollToNode` call (manual jump from bookmark/search)
- `AgentFooter` has NO `onInput` handler at all — PR #345 deliberately removed it to kill the autoGrow layout thrashing. The textarea is uncontrolled; the DOM owns the value.

So if the user scrolls up, then starts typing, `autoScroll` stays `false` and new content arrives offscreen.

### 3.2 Target behavior

When the user types in the composer, the document **immediately scrolls to the bottom once** and re-enables `autoScroll` so any subsequent content stays in view.

### 3.3 Implementation

**Three changes:**

#### 3.3.1 `AgentDocumentView.tsx` — expose a `scrollToBottomRef`

Pattern matches the existing `scrollToNodeRef`:

```ts
interface AgentDocumentViewProps {
    // … existing props
    scrollToNodeRef?: (fn: (nodeId: string) => void) => void;
    scrollToBottomRef?: (fn: () => void) => void;  // NEW
}
```

Inside the component:

```ts
const scrollToBottom = () => {
    if (!scrollRef) return;
    autoScroll = true;
    // scrollTo with an absurdly large target lets the browser clamp to
    // max during its own render pipeline. We never JS-read scrollHeight,
    // so there's no forced synchronous layout on the keystroke path.
    scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "instant" });
};

// Expose on mount
if (scrollToBottomRef) scrollToBottomRef(scrollToBottom);
```

**Why `Number.MAX_SAFE_INTEGER` instead of `scrollHeight`:** reading `scrollHeight` forces a synchronous layout flush (exactly the autoGrow bug). Passing a too-large target lets Chromium clamp it during the compositor pass without any JS-side layout read. The result is the same — scroll to the bottom — at zero keystroke cost.

#### 3.3.2 `AgentFooter.tsx` — add a tiny `onInput` handler

Re-introduce an `onInput` handler, but make it rigorously cheap:

```ts
interface AgentFooterProps {
    agentId: string;
    onSendMessage?: (message: string) => void;
    onTyping?: () => void;      // NEW
    loading?: boolean;
}

// RAF-debounced: the handler fires once per keystroke but the actual
// scroll only happens once per animation frame, regardless of how many
// keystrokes queued up. This keeps the keystroke critical path at one
// function call + one boolean check + (on the first keystroke of a
// frame) one RAF enqueue. No layout reads, no signal writes.
let scrollPending = false;
const handleInput = () => {
    if (!props.onTyping) return;
    if (scrollPending) return;
    scrollPending = true;
    requestAnimationFrame(() => {
        scrollPending = false;
        props.onTyping?.();
    });
};

// In the JSX:
<textarea
    ref={textareaRef}
    class="agent-input"
    placeholder={`Send message to ${props.agentId}...`}
    onKeyDown={handleKeyDown}
    onInput={handleInput}     // NEW
    rows={1}
/>
```

The comment block at the top of `AgentFooter.tsx` must be updated to note that the `onInput` handler is back **but** only does work off the critical path via RAF, and never reads layout properties. The PR #345 mistake was reading `scrollHeight`; this handler's entire body is a boolean flag + a function call.

#### 3.3.3 `agent-view.tsx` — wire them together

```ts
let scrollToBottomFn: (() => void) | null = null;

// In the JSX:
<AgentDocumentView
    // … existing props
    scrollToBottomRef={(fn) => { scrollToBottomFn = fn; }}
/>
<AgentFooter
    agentId={agentId}
    onSendMessage={handleSendMessage}
    onTyping={() => scrollToBottomFn?.()}
    loading={isLoading()}
/>
```

### 3.4 Cost analysis vs PR #345 baseline

PR #345 dropped keypress avg from 45 ms to 0.10 ms by deleting `autoGrow` and its forced `scrollHeight` read. Reintroducing an `onInput` handler is scary in that context. **But:**

| Operation | Cost | Layout read? |
|---|---|---|
| `handleInput` function call | <1 μs | No |
| Boolean check `scrollPending` | <1 μs | No |
| `requestAnimationFrame(...)` enqueue | <10 μs | No |
| RAF callback boolean reset | <1 μs | No |
| `scrollRef.scrollTo({top: MAX, behavior: "instant"})` call | ~100-500 μs | No (browser clamps internally) |

Total per-keystroke cost: **well under 1 ms**, and the RAF debounce means the `scrollTo` only runs once per animation frame even under sustained typing. Compare PR #345's baseline:

- Before #345: 45.12 ms avg (autoGrow layout thrash)
- After #345: 0.10 ms avg (no onInput)
- After this spec: estimated 0.3-1.0 ms avg (cheap onInput + RAF-debounced scroll)

Still 30-150× faster than the pre-#345 regression. Still in the "feels instant" range. A fresh CDP trace post-implementation will confirm this; target is **keypress avg < 2 ms**.

---

## 4. Files changed

| File | Change | Est. lines |
|---|---|---|
| `frontend/app/view/agent/components/ToolBlock.tsx` | `overlayUp` signal, `handleMouseEnter`/`handleMouseLeave`, `findScrollParent` helper, `overlay-mode` / `overlay-up` class application | +35 / -5 |
| `frontend/app/view/agent/agent-view.scss` | `.agent-tool-block { position: relative }`, `.overlay-mode .agent-tool-content` overlay rules, `.overlay-up` flip variant | +30 / -5 |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | `scrollToBottomRef` prop, `scrollToBottom` local function, expose on mount | +20 / -0 |
| `frontend/app/view/agent/components/AgentFooter.tsx` | Re-introduce `handleInput` (RAF-debounced), `onTyping` prop, updated top-of-file comment | +20 / -5 |
| `frontend/app/view/agent/agent-view.tsx` | `scrollToBottomFn` ref, wire `onTyping`, wire `scrollToBottomRef` | +6 / -0 |

**Total:** ~110 lines changed across 5 files.

---

## 5. Test plan

### 5.1 Manual (in the 0.33.109 portable built from this branch)

**Tool overlay:**
- [ ] Open agent pane with ≥5 tool blocks visible
- [ ] Hover the first tool. The expanded content appears as a floating panel over the tools below. Tool #2, #3, #4 stay in place.
- [ ] Move the mouse from the summary down into the overlay. The overlay stays open.
- [ ] Move the mouse out of the overlay entirely. It closes immediately.
- [ ] Hover a tool near the **bottom** of the visible area. The overlay flips up (`overlay-up` class applied) and appears above the summary row.
- [ ] Click a tool to pin it. The overlay stays. Hover another tool. Both are visible (pinned overlay + hover overlay). Click the pinned tool again. It collapses.
- [ ] A running tool renders its content **inline** (not as overlay). Same for a failed tool.

**Scroll-on-type:**
- [ ] Open an agent pane with a long session. Scroll up so the latest content is offscreen.
- [ ] Type a single character in the composer. The document scrolls immediately to the bottom.
- [ ] Continue typing. The document stays at the bottom; auto-scroll is re-enabled.
- [ ] Scroll up manually. Stop typing. Auto-scroll stays off (scroll listener disables it).
- [ ] Type again. Document scrolls to bottom once more.

### 5.2 Automated (CDP trace)

Capture a fresh CDP trace while typing 30 characters into the composer (`capture-trace.cjs` and `verify-typing-fix.cjs` were removed — use Chrome DevTools → Performance tab or a CDP `Profiler.start` session directly). Target:

- keypress avg < 2 ms (vs 0.10 ms baseline — allow a ~20× slack for the new RAF path, still orders of magnitude better than the 45 ms pre-#345 regression)
- No new Layout events during keypresses beyond the pre-fix baseline
- No forced synchronous layouts (no `FunctionCall → Layout` nesting in the flame graph)

If the trace shows keypress avg ≥ 5 ms, the RAF path is too expensive and we need to debounce further (e.g. only scroll on the first keystroke of a "typing burst," not every frame).

### 5.3 Smoke test

Manual smoke test against the target build (`smoke-test-portable.cjs` was removed — verify manually: launch portable, open an agent pane, type into the composer, confirm tool overlay renders and scroll-on-type behaves). Must show no regression from pre-fix baseline.

---

## 6. Principles enforced

1. **The document is a stable surface.** Nothing in the agent pane causes it to shift under the user — not hover, not click, not state transitions. The only time the document moves is when the user explicitly asks for it (scrolling, typing, or auto-scroll re-enabled after typing).
2. **Overlays are DOM descendants of their anchor.** Absolute positioning + DOM parent = correct `mouseenter`/`mouseleave` semantics without timers.
3. **No layout reads on the keystroke hot path.** `scrollHeight` is still banned in `onInput`. `scrollTo({top: MAX_SAFE_INTEGER})` is the sanctioned replacement.
4. **RAF-debounce side effects that want to react to every keystroke.** One function call per frame is fine; one per keystroke is premature and will accumulate cost in long typing bursts.
5. **Transient affordances (hover) overlay; persistent state (running/failed) stays inline.** Consistent rule: things the user is momentarily looking at float; things that represent ongoing state take space.

---

## 7. Out of scope

- **AgentControlBar's expanded body** still pushes the document down when the chevron is clicked. That's a deliberate interaction — the user explicitly toggles it. Leave it as-is.
- **BookmarksPanel**, **AgentSearchBar**, **SessionDigestBanner** — same story. They mount when the user toggles a feature, not on passive hover. A future PR could convert them to overlays too if desired, but this spec covers only the two reported bugs.
- **Bottom-flip edge case for very tall tool content.** If the overlay's max-height (400px) is still too big for the available flip space, we just accept internal scrolling within the overlay. No double-flip logic.
- **Touch devices.** Touch has no hover; click-to-pin is the only interaction. Overlay still applies via the pinned state. No changes needed.
- **Changes to the 4 banners in AgentControlBar (interrupted, large-session, archived, digest).** Separate spec.

---

## 8. Rollout

Single PR, single bump (→ 0.33.109), single build. Both fixes are small, related, and share a test plan. Merge order: this PR → build portable → manual test per §5.1 → capture trace per §5.2 → ship.

Estimated effort: **90 minutes** end-to-end including build + verification.
