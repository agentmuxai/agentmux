# SPEC: Tool Block Interaction Hold + Glob Auto-Expand

**Date:** 2026-06-09  
**Status:** Implemented  
**Files changed:**
- `frontend/app/view/agent/components/ToolBlock.tsx`
- `frontend/app/view/agent/components/CompactResult.tsx`

---

## Problem

### 1. Post-completion collapse interrupts active reading

When a tool completes, `ToolBlock` holds the panel open for `POST_COMPLETION_HOLD_MS` (3s) then collapses. If the user's mouse is inside the block while the timer fires — actively reading the output — the panel collapses under them with no way to prevent it short of clicking to pin.

### 2. Glob results always start collapsed

`CompactResult` initializes with `createSignal(false)` for all tools. Glob results are therefore collapsed by default, showing only a one-line path summary. Users must click the chevron to see the full file list every time — high friction for a result that is almost always worth reading in full.

---

## Changes

### ToolBlock.tsx — interaction hold

Added a `userHolding` signal that latches when the user's mouse enters an already-expanded block, and clears on mouse leave.

```ts
const [userHolding, setUserHolding] = createSignal(false);
const expanded = () => props.pinned || autoExpanded() || userHolding();
```

Mouse handlers on the outer `agent-tool-block` div:

```tsx
onMouseEnter={() => { if (props.pinned || autoExpanded()) setUserHolding(true); }}
onMouseLeave={() => setUserHolding(false)}
```

**Behavior:**
- Mouse enters while block is auto-expanded (running / post-completion hold) → `userHolding` latches true. Post-completion timer can now fire and clear `autoExpanded` without collapsing the block.
- Mouse leaves → `userHolding` clears → block collapses normally.
- Mouse enters a *collapsed* block → guard (`props.pinned || autoExpanded()`) is false → `userHolding` stays false → no change. Hover-to-peek remains removed per `SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md`.

**Not affected:** pin behavior, the post-completion timer itself, or any keyboard/a11y path.

### CompactResult.tsx — Glob default expanded

```ts
// Before
const [expanded, setExpanded] = createSignal(false);

// After
const [expanded, setExpanded] = createSignal(tool === "Glob");
```

Glob results render with the full file list visible on mount. All other tools remain collapsed by default. The toggle chevron still works — users can collapse a Glob result manually.

---

## Design notes

- The `userHolding` guard on mouse-enter (`if (props.pinned || autoExpanded())`) is intentional: it prevents hover from opening an already-collapsed tool. This preserves the consolidation from `SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md` — collapsed tools stay collapsed on hover.
- `userHolding` is not persisted in `documentState.pinnedNodes`. It is a transient, per-mount interaction signal. If the component unmounts and remounts (virtualization scroll-off), the block starts fresh with no hold. This is acceptable — the user can re-hover or click to pin.
- Glob default-expand applies at the `CompactResult` level, so it takes effect wherever `CompactResult` is used with `tool="Glob"` (currently only `ToolOverlayLog.tsx`).
