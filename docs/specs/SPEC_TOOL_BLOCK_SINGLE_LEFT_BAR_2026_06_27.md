# SPEC: Tool Block Single Left Bar

**Date:** 2026-06-27  
**Status:** Proposed  
**Area:** Agent pane — tool call rendering

---

## Problem

Tool call blocks in the agent pane (Read, Bash, Shell, etc.) currently display **two vertical lines** on the left:

1. **Outer bar** — on `.agent-tool-block` (`border-left: 2px solid var(--border-color)`, line 138 of `_document-nodes.scss`). Color changes per status: warning/success/error/secondary/accent(pinned). This is the bar flush with the left pane edge.

2. **Inner bar** — on `.agent-tool-panel` (`border-left: 2px solid var(--accent-color, #2bd4a8)`, line 280). It sits 24px inset from the outer bar (due to `margin-left: 24px` on line 279), creating a second visual rail next to the expanded content.

The inner bar is redundant: the outer bar already communicates tool status and identity. The inner bar adds visual noise and unnecessary indentation.

---

## Current State (as of v0.49.6)

File: `frontend/app/view/agent/styles/_document-nodes.scss`

```scss
// Line 132–158
.agent-tool-block {
    border-left: 2px solid var(--border-color);   // OUTER bar — KEEP
    ...
    &.running  { border-left: 2px solid var(--warning-color); }
    &.success  { border-left: 2px solid var(--success-color); }
    &.failed   { border-left: 2px solid var(--error-color); }
    &.canceled { border-left: 2px solid var(--secondary-text-color); }
    &.pinned   { border-left: 2px solid var(--accent-color); }   // line 248

    // Line 276–341
    .agent-tool-panel {
        margin: 4px 0 8px 24px;                              // 24px left indent
        border-left: 2px solid var(--accent-color, #2bd4a8); // INNER bar — REMOVE
        padding: 6px 8px;
        ...
    }
}
```

Visual layout (current):
```
│ ← outer bar (border-color / status-color)
│   [summary row: icon  tool-name  args  duration]
│
│                    │ ← inner bar (accent-color, 24px in)
│                    │  file content / tool output text
│                    │
```

---

## Desired State

Remove the inner bar. Shift the expanded panel content left so it is flush with the outer bar (left edge of `.agent-tool-block`). The outer bar alone carries the status signal.

Visual layout (target):
```
│ ← outer bar (border-color / status-color)
│ [summary row: icon  tool-name  args  duration]
│
│ file content / tool output text
│
```

---

## Change

**File:** `frontend/app/view/agent/styles/_document-nodes.scss`

**In `.agent-tool-panel` (lines 276–283):**

| Property | Before | After |
|----------|--------|-------|
| `margin` | `4px 0 8px 24px` | `4px 0 8px 0` |
| `border-left` | `2px solid var(--accent-color, #2bd4a8)` | *(remove)* |
| `padding` | `6px 8px` | `6px 8px` *(unchanged)* |

Diff:
```scss
.agent-tool-panel {
    display: flex;
    flex-direction: column;
-   margin: 4px 0 8px 24px;
+   margin: 4px 0 8px 0;
-   border-left: 2px solid var(--accent-color, #2bd4a8);
    background: var(--surface-elevated, rgba(255, 255, 255, 0.02));
    border-radius: 0;
    padding: 6px 8px;
    ...
}
```

No changes needed to `.agent-tool-block`, `.agent-tool-summary`, or any other rule. The `--hidden` and `--flow` modifiers are unaffected (they override `max-height`, `padding`, `margin`, `opacity` — removing `border-left` and resetting `margin-left` to `0` doesn't interact with those).

---

## Side Effects / Risks

- **Comment on line 272–274** references "faint accent border-left to tie it visually to the running/streaming state" — this comment should be removed or updated since the border is gone.
- **`pinned` state** (`&.pinned { border-left: 2px solid var(--accent-color); }` on `.agent-tool-block`, line 248) is unaffected — it overrides the outer bar, not the inner panel.
- **Transition** on `.agent-tool-panel` animates `margin` (line 301) — removing `margin-left: 24px` means the collapse animation still works correctly (it transitions `margin-top` and `margin-bottom` to 0 in `--hidden`; `margin-left: 0` is the same as before the hidden state, so no visual regression).
- The `background: var(--surface-elevated)` tint on `.agent-tool-panel` still visually distinguishes the expanded content from the summary row without needing the second border.

---

## Files Touched

| File | Change |
|------|--------|
| `frontend/app/view/agent/styles/_document-nodes.scss` | Remove `border-left` + set `margin-left: 0` on `.agent-tool-panel`; update adjacent comment |

No TypeScript, no Rust, no other SCSS files.

---

## Verification

1. Open an agent pane with an expanded Read/Bash/Shell tool call block.
2. Confirm only one vertical bar is visible on the left.
3. Confirm the expanded content text is flush with that bar (not indented 24px inward).
4. Confirm collapse/expand animation still works (120ms ease-out).
5. Confirm pinned blocks still show accent outer bar.
6. Confirm running/success/failed/canceled color states still apply to the outer bar.
