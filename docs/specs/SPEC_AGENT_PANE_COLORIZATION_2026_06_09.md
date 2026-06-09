# SPEC: Agent Pane Output Colorization

**Date:** 2026-06-09  
**Status:** Draft  
**Scope:** CSS-only changes + one JSX attribute addition  
**Files touched:** `frontend/app/view/agent/styles/_document-nodes.scss`, `frontend/app/view/agent/styles/_tool-overlay-portal.scss`, `frontend/app/view/agent/components/ToolBlock.tsx`

---

## Problem

The agent pane output is visually monochromatic. Almost all rendered content —
tool names, bash output, assistant prose, streaming chunks, section headings —
uses `--main-text-color` (#f7f7f7). The colors that DO exist are limited to
narrow decorative chrome: 2 px left-border status stripes and small status
icons. A user reading the conversation sees a wall of identically-colored white
text with no visual hierarchy beyond indentation.

### Verified flat/colorless surfaces (cross-referenced against source)

| Surface | Class | Verified location | Current color |
|---|---|---|---|
| Tool name in summary row | `.agent-tool-name` | `_document-nodes.scss:196–203` | inherited `--main-text-color` (no `color` rule) |
| Streaming stdout chunks | `.agent-tool-log-line--stdout` | `_tool-overlay-portal.scss:41–58` | **no CSS rule at all** |
| Streaming diff-hunk chunks | `.agent-tool-log-line--diff` | `_tool-overlay-portal.scss:41–58` | **no CSS rule at all** |
| Bash stdout body | `.agent-bash-output` (non-stderr) | `_document-nodes.scss:544–556` | inherited (no `color` rule — only `.has-error` / `.agent-bash-stderr` sub-rules have color) |
| Bash command text | `.agent-bash-cmd-code` | `_document-nodes.scss:523–541` | inherited (only `.agent-bash-dollar` has `--accent-color`) |
| Section headings h2/h3 | `.agent-section.level-2/3 h2/h3` | `_document-nodes.scss:982–993` | inherited (h1 has a `border-bottom` but no `color`) |
| Thinking block body text | `.agent-markdown-block.thinking-block` body | `_document-nodes.scss:46–53` | `--main-text-color` at 60% opacity (same hue as prose) |
| Compact result summary | `.agent-tool-compact-summary` | `_document-nodes.scss:1069–1075` | `--main-text-color` at 85% opacity |

### What already has correct color (do not change)

- Tool left-border status stripes (yellow/green/red) — correct
- Status icon tints (`.agent-tool-status-icon` per-state) — correct
- `$` prompt prefix (`.agent-bash-dollar` → `--accent-color`) — correct
- Bash stderr body (`.agent-bash-stderr` → `--error-color`) — correct
- Bash exit code row (`.agent-bash-exit` → green/red) — correct
- Diff add/del/hunk lines — correct
- Streaming stderr chunks (`.agent-tool-log-line--stderr` → `--error-color`) — correct
- User message block (cyan `--user-input-color` background + border) — correct
- Read/Write/Search `.agent-tool-file-path` → `--accent-color` — correct
- Live-tail inline preview (`.agent-tool-live-tail` → `--accent-color`) — correct

---

## Color Palette

All colors are **existing CSS variables from `theme.scss`**. No new hex literals.

| Token | Value | Semantic meaning in this spec |
|---|---|---|
| `--term-bright-green` | `#58c142` | Bash/execution (terminal green = running a command) |
| `--warning-color` | `rgb(224, 185, 86)` | Write/Edit (file mutation = deserves attention) |
| `--term-bright-cyan` | `#34e2e2` | Grep/Glob (search/find) |
| `--term-bright-magenta` | `#ad7fa8` | Agent (agent orchestration / AI spawn) |
| `--accent-color` | `rgb(65, 159, 224)` | Read / section h1 (informational, already used for file paths) |
| `--term-foreground` | `#d3d7cf` | Streaming stdout (slightly muted from pure white — readable but distinct from prose) |
| `--secondary-text-color` | `rgb(195, 200, 194)` | Task / Other / thinking body (deprioritised metadata) |

---

## Changes

### Change 1 — `data-tool` attribute on `.agent-tool-block` (JSX)

**File:** `frontend/app/view/agent/components/ToolBlock.tsx`  
**Lines:** 143–152 (the outer `<div class={clsx("agent-tool-block", {...})}>`)

Add `data-tool={props.node.tool.toLowerCase()}` to the outer div so CSS can
target per-tool color without adding runtime class string logic.

The `tool` field type is `"Read" | "Edit" | "Bash" | "Write" | "Grep" | "Glob" | "Task" | "Agent" | "Other"` (verified: `types.ts:213`). Lowercasing is applied at the attribute so CSS selectors are lowercase. The `"Other"` value lowercases to `"other"`.

**Before:**
```tsx
<div
    class={clsx("agent-tool-block", {
        collapsed: !expanded(),
        ...
    })}
>
```

**After:**
```tsx
<div
    class={clsx("agent-tool-block", {
        collapsed: !expanded(),
        ...
    })}
    data-tool={props.node.tool.toLowerCase()}
>
```

---

### Change 2 — Per-tool `.agent-tool-name` color rules (CSS)

**File:** `frontend/app/view/agent/styles/_document-nodes.scss`  
**Insert after:** the `.agent-tool-summary` block (after line ~233)

```scss
// Per-tool-type color on the collapsed summary name.
// Uses data-tool attribute added to .agent-tool-block in ToolBlock.tsx.
// All tokens are from theme.scss — no new hex literals.
.agent-tool-block[data-tool="bash"]   .agent-tool-name { color: var(--term-bright-green); }
.agent-tool-block[data-tool="read"]   .agent-tool-name { color: var(--accent-color); }
.agent-tool-block[data-tool="write"]  .agent-tool-name { color: var(--warning-color); }
.agent-tool-block[data-tool="edit"]   .agent-tool-name { color: var(--warning-color); }
.agent-tool-block[data-tool="grep"]   .agent-tool-name { color: var(--term-bright-cyan); }
.agent-tool-block[data-tool="glob"]   .agent-tool-name { color: var(--term-bright-cyan); }
.agent-tool-block[data-tool="agent"]  .agent-tool-name { color: var(--term-bright-magenta); }
.agent-tool-block[data-tool="task"]   .agent-tool-name { color: var(--secondary-text-color); }
.agent-tool-block[data-tool="other"]  .agent-tool-name { color: var(--secondary-text-color); }
```

**Constraint:** These rules must sit OUTSIDE the `.agent-tool-block { ... }` rule block
so the specificity of `[data-tool]` attribute selector beats the base `.agent-tool-block`
hover rules. Place them immediately after the closing `}` of `.agent-tool-block`.

---

### Change 3 — Bash command text color (CSS)

**File:** `frontend/app/view/agent/styles/_document-nodes.scss`  
**Target:** `.agent-bash-cmd-code` block (currently lines 523–541)

The `$` prefix already uses `--accent-color`. The command text next to it
should match so the entire command line reads as a unit.

```scss
.agent-bash-cmd-code {
    // ... existing rules unchanged ...
    color: var(--accent-color);   // ADD THIS LINE
}
```

---

### Change 4 — Streaming stdout and diff-hunk chunk colors (CSS)

**File:** `frontend/app/view/agent/styles/_tool-overlay-portal.scss`  
**Target:** `.agent-tool-log-line` block (lines 41–58)

Two modifier classes currently map from `KIND_CLASS` in `ToolOverlayLog.tsx`
but have no CSS rules:
- `agent-tool-log-line--stdout` (kind `"stdout"`)
- `agent-tool-log-line--diff` (kind `"diff-hunk"`)

```scss
.agent-tool-log-line {
    // ... existing rules unchanged ...

    &--stdout {
        color: var(--term-foreground);   // #d3d7cf — slightly muted from main-text
    }
    &--diff {
        color: var(--accent-color);
        opacity: 0.9;
    }
    // --stderr already has color: var(--error-color) — unchanged
    // --system already has font-style: italic + opacity — unchanged
}
```

**Why `--term-foreground` for stdout:** It is `#d3d7cf` — visually distinct from
the prose `--main-text-color` (#f7f7f7) but still clearly readable. It matches
the colour used in the xterm terminal surface (deliberate: streaming output
feels like a terminal).

---

### Change 5 — Section heading colors (CSS)

**File:** `frontend/app/view/agent/styles/_document-nodes.scss`  
**Target:** `.agent-section` block (lines 969–994)

```scss
&.level-1 h1 {
    // ... existing rules ...
    color: var(--accent-color);   // ADD: accent blue for top-level section titles
}
&.level-2 h2 {
    // ... existing rules ...
    color: var(--main-text-color);  // explicit (already inherited, but clear intent)
    opacity: 0.9;
}
// level-3 h3 unchanged — already has opacity: 0.8
```

---

### Change 6 — Thinking block body text color (CSS)

**File:** `frontend/app/view/agent/styles/_document-nodes.scss`  
**Target:** `.agent-markdown-block.thinking-block` block (lines 46–53)

Currently the thinking block is italic + 60% opacity but the text is still the
same white hue as the main response text. Adding a slight secondary-text tint
makes it unambiguously "not the final answer".

```scss
&.thinking-block {
    // ... existing rules ...
    color: var(--secondary-text-color);   // ADD: distinguishes thinking from prose
}
```

---

## What is NOT changed

- **Assistant prose markdown** (`.agent-markdown-block`) — keeps `--main-text-color`
  at 90% opacity. It is the primary content surface; giving it a tint would
  reduce its prominence relative to tool output.
- **User message** (`.agent-user-message`) — already has distinct cyan treatment.
- **Diff add/del/hunk/ctx** — already colored correctly; no change.
- **Tool status left-borders** — already correct; no change.
- **`.agent-tool-file-path`** in Read/Write/Search panels — already `--accent-color`.
- **Compact result** (`.agent-tool-compact-summary`) — left at `--main-text-color`
  85% opacity. It is a collapsed/secondary surface; colorizing it risks visual
  noise when many tools terminate.
- **Bash stdout body** (`.agent-bash-output` non-stderr) — left as inherited. The
  streaming chunks path (`--stdout`) already gets `--term-foreground` (Change 4).
  Bash output uses the structured `BashOutputViewer` only post-completion; most
  live output goes through streaming chunks. Post-completion body stays white to
  match the xterm terminal surface.

---

## Implementation order

1. **ToolBlock.tsx** — add `data-tool` attribute (unblocks all CSS tool-name rules)
2. **`_document-nodes.scss`** — Changes 2, 3, 5, 6
3. **`_tool-overlay-portal.scss`** — Change 4

All changes are additive (new rules or new properties on existing rules).
No existing color rules are removed or overridden (except Change 3 adding
`color` to `.agent-bash-cmd-code` which currently has none).

---

## Verification checklist

- [ ] Bash tool name shows in `--term-bright-green` (#58c142)
- [ ] Read tool name shows in `--accent-color` (rgb(65, 159, 224))
- [ ] Edit/Write tool name shows in `--warning-color` (rgb(224, 185, 86))
- [ ] Grep/Glob tool name shows in `--term-bright-cyan` (#34e2e2)
- [ ] Agent tool name shows in `--term-bright-magenta` (#ad7fa8)
- [ ] Bash command text (`.agent-bash-cmd-code`) matches the `$` prefix blue
- [ ] Streaming stdout chunks render in `--term-foreground` (#d3d7cf)
- [ ] Streaming diff-hunk chunks render in `--accent-color`
- [ ] Section h1 renders in `--accent-color`
- [ ] Thinking block text is `--secondary-text-color` (distinguishable from prose)
- [ ] Existing diff add/del colors unchanged
- [ ] Existing user message cyan unchanged
- [ ] Existing status icon colors unchanged
- [ ] Existing left-border status stripes unchanged
