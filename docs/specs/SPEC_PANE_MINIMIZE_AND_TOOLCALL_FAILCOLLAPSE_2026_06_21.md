# SPEC — Pane Minimize Button + Failed Tool Call Immediate Collapse

**Date:** 2026-06-21
**Status:** Proposed

---

## Overview

Two independent UI tweaks shipped in one PR:

1. **Failed tool call immediate collapse** — when a tool call finishes with status `failed`, `denied`, or `canceled`, collapse it immediately instead of holding it open until it scrolls off screen.
2. **Pane minimize button** — add a minimize icon to the pane header that rolls the block content up to the header-only strip. Also separate the aux buttons from the standard window-management triad (min/max/close) with a light vertical pipe.

---

## Feature 1 — Failed Tool Call Immediate Collapse

### Current behaviour

`ToolBlock.tsx` holds a block open after the active→inactive transition via the `heldOpen` prop (driven by scroll position in `AgentDocumentVirtualList`). This lets the user read fresh output. The hold lifts only when the row scrolls off the top of the viewport.

### Desired behaviour

For terminal-failure states (`failed`, `denied`, `canceled`), skip the hold entirely — collapse to the compact pill immediately on transition. The user can still click to expand manually. Successful runs keep the current hold behaviour.

### What to change

**`frontend/app/view/agent/components/ToolBlock.tsx`**

The `autoExpanded` memo (around line 124) currently expands when status is `running` or `pending_approval`. The expanded state is:

```ts
const expanded = () =>
    props.pinned || autoExpanded() || (heldOpen && props.heldOpen);
```

Add a short-circuit: if the status is a failure terminal (`failed | denied | canceled`), treat `heldOpen` as false regardless of the prop:

```ts
const isFailTerminal = () =>
    props.node.status === "failed" ||
    props.node.status === "denied" ||
    props.node.status === "canceled";

const expanded = () =>
    props.pinned || autoExpanded() || (!isFailTerminal() && props.heldOpen);
```

No changes to `AgentDocumentVirtualList` — it can still clear `heldOpen` normally; we just ignore it locally for fail-terminal rows.

### CSS

No change needed — `.agent-tool-panel--hidden` already handles the collapsed visual.

---

## Feature 2 — Pane Minimize Button + Header Separator

### 2a. Minimize button

#### Behaviour

- Click minimize → block content area rolls up; only the header strip (32px) remains visible.
- Click minimize again → content restores to previous height.
- State persists per block via block meta key `layout:minimized` (boolean, default `false`).
- When minimized, the block frame root gets class `block-frame--minimized`; the content area's `overflow: hidden` + `height: 0` (or `max-height: 0`) hides it.
- Minimized state does NOT affect layout sizing of neighbouring panes — the shell block collapses in place; grid tracks remain their natural sizes. This is purely a visual hide (content-visibility).

#### Symbol

Use `—` (em-dash, U+2014) as the minimize glyph, matching platform conventions. Alternatively `window-minimize` icon if available in the icon set; fall back to a short horizontal bar `▁` or inline SVG. Check the icon registry first.

#### Where to add

**`frontend/app/block/blockframe.tsx`**

In `BlockFrame_Header()`, add a `minimizeButton` between `endIconButtons` and `magnifyButton`:

```tsx
const minimizeButton = (
    <IconButton
        decl={{
            elemtype: "iconbutton",
            icon: "window-minimize",   // or inline SVG bar
            title: nodeModel.minimizedAtom()
                ? "Restore pane"
                : "Minimize pane",
            click: () => void nodeModel.toggleMinimized(),
        }}
        className="block-frame-minimize"
    />
);
```

Standard triad order in `block-frame-end-icons`: **[aux buttons] | [minimize] [magnify] [close]**

#### State & persistence

Add to the node model (likely `frontend/app/block/blockframemodel.tsx` or `WaveObjectModel` — wherever `toggleMagnify` lives):

```ts
const META_MINIMIZED = "layout:minimized";

private _minimized = createSignal<boolean>(false);
minimizedAtom: Accessor<boolean> = this._minimized[0];

// Hydrate from block meta in constructor (same pattern as editor tree state):
if (meta?.[META_MINIMIZED] === true) this._minimized[1](true);

async toggleMinimized(): Promise<void> {
    const next = !this._minimized[0]();
    this._minimized[1](next);
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref: makeORef("block", this.blockId),
        meta: { [META_MINIMIZED]: next || null },  // null removes key when false
    });
}
```

The block frame root element gets a reactive class:

```tsx
<div
    class="block-frame-default"
    classList={{ "block-frame--minimized": nodeModel.minimizedAtom() }}
    ...
>
```

#### CSS

```scss
.block-frame--minimized {
    .block-frame-content {
        height: 0;
        overflow: hidden;
        content-visibility: hidden;
    }
    // Keep the header fully visible and interactive
    .block-frame-default-header {
        border-bottom: none;
    }
}
```

`block-frame-content` is the existing wrapper that holds the view below the header — confirm the actual class name in `block.scss` before applying.

### 2b. Separator between aux buttons and standard triad

#### Behaviour

A subtle 1px vertical rule between the aux icon buttons (block-type-specific) and the standard window-management buttons (minimize, magnify, close). Only shown when aux buttons are present.

#### Implementation

**`frontend/app/block/blockframe.tsx`** — in the `block-frame-end-icons` section, add a separator element conditionally:

```tsx
<div class="block-frame-end-icons">
    {endIconButtons}
    <Show when={endIconButtons()?.length > 0}>
        <div class="block-frame-btn-separator" aria-hidden="true" />
    </Show>
    {voiceHandle}
    {minimizeButton}
    {magnifyButton}
    {closeButton}
</div>
```

#### CSS

```scss
.block-frame-btn-separator {
    width: 1px;
    height: 14px;          // shorter than the icon, centered vertically
    background: var(--border-color);
    opacity: 0.5;
    align-self: center;
    margin: 0 3px;
    flex-shrink: 0;
}
```

---

## Files touched

| File | Change |
|---|---|
| `frontend/app/view/agent/components/ToolBlock.tsx` | Add `isFailTerminal()`, short-circuit `heldOpen` in `expanded()` |
| `frontend/app/block/blockframe.tsx` | Add `minimizeButton` JSX, separator, reactive `block-frame--minimized` class |
| `frontend/app/block/blockframemodel.tsx` (or equivalent) | Add `_minimized` signal, `minimizedAtom`, `toggleMinimized()`, meta hydration |
| `frontend/app/block/block.scss` | Add `.block-frame--minimized`, `.block-frame-btn-separator` |

---

## Out of scope

- Keyboard shortcut for minimize (follow-on).
- Minimize affecting grid track height (complex layout change — separate spec).
- Minimize state in tab strip thumbnails.
- Restoring scroll position inside the content on un-minimize (browser handles this naturally with `height` animation).
