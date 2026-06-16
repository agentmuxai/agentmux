# What is the hover popup on an agent tool/output line?

**Date:** 2026-06-15
**Answer in one line:** It's the **`NodeHoverStrip`** — a per-row, CSS-only **hover strip** (not a tooltip, not a JS popover) that fades into the **top-right of each conversation row** and shows the line's **timestamp** plus an **expand/collapse** button.

---

## What it is

| | |
|---|---|
| Component | `frontend/app/view/agent/components/NodeHoverStrip.tsx` (renders `<div class="node-strip">`) |
| Rendered by | `frontend/app/view/agent/virtualization/DocumentRow.tsx:136` (every conversation row) and `components/ActivityLogPanel.tsx:90` (activity-log rows) |
| Styled in | `frontend/app/view/agent/styles/_document.scss` (`.node-strip`, ~line 41) |
| Shows on hover | Yes — but via **pure CSS**, no JS show/hide signal |

It is **not** a native `title=` tooltip and **not** a floating portal popup. It's an absolutely-positioned element that lives *inside* each row wrapper and is revealed on hover.

## What it contains

1. **Timestamp** (`<time class="node-strip-time">`) — localized via `Intl.DateTimeFormat`: weekday + month + day + 12-hour time with seconds, and the **year is added once the line is ≥ 7 days old** (`NodeHoverStrip.tsx:92`). Tabular-nums so it doesn't jitter.
2. **Expand / collapse button** (`⊞` / `⊟`) — only when the node `canExpand` (e.g. a collapsible tool block); toggles the row's expanded state via `onExpand`.

That's the whole strip. Branching actions (open-in-pane / open-in-window / new-agent-here) used to be candidates for this strip but were **moved into the tool overlay's bottom action bar** so each row's hover chrome stays lean (see the comment in `NodeHoverStrip.tsx:16` and `SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.5`).

## How the hover works (why it reads like a popup)

From `_document.scss`:

```scss
.agent-document-node-wrapper {            // the shared hover anchor (position: relative)
    .node-strip {
        position: absolute;
        top: 3px; right: 6px;             // top-right corner of the row
        background: var(--main-bg-color); // fully opaque (panel bg is 50% alpha — would bleed)
        border: 1px solid var(--border-color);
        box-shadow: 0 1px 3px rgba(0,0,0,0.15);
        opacity: 0;                       // hidden by default
        pointer-events: none;
        transition: opacity 80ms ease;
        z-index: 6;
    }
    &:hover .node-strip,
    &:focus-within .node-strip { opacity: 1; }   // reveal on hover OR keyboard focus
}
```

So:
- **Trigger:** hovering the row (`:hover`) *or* focusing it with the keyboard (`:focus-within`) — accessible, not mouse-only.
- **Animation:** an 80ms opacity fade — which is why it feels like a little popup appearing.
- **Anchoring:** pinned to the row's top-right with an opaque background + border + soft shadow, so it visually floats over the line content (hence "popup/tooltip"), but it's an in-row strip, not an overlay layer.
- **Pointer events:** the strip itself is `pointer-events: none`; only its buttons (`.node-strip-btn`) re-enable pointer events, so hovering the strip area doesn't block text selection on the row.

## Not to be confused with

- **The tool overlay** (`styles/_tool-overlay-portal.scss`, `ToolBlock.tsx`) — a richer, separately-triggered expanded view of a tool's live log/output with its own action bar. That's the *expanded* tool surface; the hover strip is just the lightweight per-row affordance that (among other things) toggles it open.
- Native browser `title=` tooltips on individual buttons (the strip's buttons set `title`/`aria-label`, which produce OS tooltips on their own — a separate, secondary thing).

## TL;DR
The thing you see on hover is the **NodeHoverStrip / `.node-strip`**: a CSS-revealed strip in each row's top-right showing the **timestamp** and an **expand toggle**. Reusable across the conversation document and the activity log; no JS hover state; revealed on `:hover`/`:focus-within` with an 80ms fade.
