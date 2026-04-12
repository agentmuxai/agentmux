# SPEC: Tool Call Collapsed-by-Default with Hover Expand

## Problem

Tool call blocks in both the **AI Chat Panel** (`aitooluse.tsx`) and **Agent View** (`ToolBlock.tsx`) currently display expanded, taking up significant vertical space. When an agent runs many tools (Read, Grep, Edit, Bash, etc.), the chat/session becomes dominated by tool output rather than the agent's reasoning and responses.

Users need to see *that* a tool ran and *whether it succeeded* — not the full output by default.

## Goal

Tool calls should render as a **single collapsed line** by default and **expand on mouse hover**.

## Design

### Collapsed State (Default — 1 line)

```
<StatusIcon> <ToolName> <StatusCheck> <Ellipsis>
```

Examples:
```
✓ Read                    ...
✓ Edit                    ...
✗ Bash                    ...
⏳ Grep                    ...
✓ Reading Files (3)       ...
```

Layout:
- **StatusIcon**: `✓` (green, completed), `✗` (red, error), `⏳` (gray, pending/running)
- **ToolName**: Tool name or summary (e.g. `Read`, `Edit`, `Bash`, or batch label like `Reading Files (3)`)
- **StatusCheck**: Inline with tool name — no separate column needed, the icon IS the status
- **Ellipsis** (`...`): Right-aligned or trailing, signals expandable content. Styled `text-gray-500`.
- **Single line height**: No wrapping. `overflow: hidden; white-space: nowrap; text-overflow: ellipsis;`
- **Background**: Subtle `bg-gray-800/50` with `border-left: 2px solid <statusColor>` for quick visual scanning
- **Cursor**: `pointer` to signal interactivity

### Expanded State (on Hover)

On `mouseenter`, after a **150ms delay** (prevents flicker on scroll-through), the block expands to show full tool content:

- **AI Chat Panel** (`AIToolUse`): Shows `tooldesc`, error messages, approval buttons
- **Agent View** (`ToolBlock`): Shows full tool-specific rendering (DiffViewer, BashOutputViewer, file content, search results, etc.)

On `mouseleave`, after a **300ms delay** (lets user move mouse into expanded content), collapses back to single line.

### Exceptions — Do NOT Auto-Collapse

These states remain expanded regardless:

1. **`needs-approval`** — User must see and interact with Approve/Deny buttons
2. **`running`/`pending`** with active streaming — Currently executing tool stays visible
3. **`error`** — Failed tools stay expanded so errors are immediately visible

Once an `error` tool is hovered and then unhovered, it can collapse (user has acknowledged it).

### Click Behavior

- **Click** toggles a **pinned** expanded/collapsed state that overrides hover behavior
- A pinned-open tool stays open even when mouse leaves
- Clicking again unpins and collapses

## Changes Required

### 1. AI Chat Panel — `frontend/app/aipanel/aitooluse.tsx`

**`AIToolUse` component (line 141):**
- Add `isExpanded` state (default: `false`)
- Add `isPinned` state (default: `false`)
- Add hover timer refs for enter/leave delays
- When collapsed: render single-line summary row only
- When expanded: render current full content (lines 226-243)
- Exception: if `effectiveApproval === "needs-approval"`, force expanded

**`AIToolUseBatch` component (line 74):**
- Same pattern: collapsed shows `"Reading Files (N) ✓ ..."` on one line
- Expanded shows current batch item list
- Exception: if any item `needs-approval`, force expanded

**`AIToolUseBatchItem` component (line 46):**
- No changes needed — only rendered inside expanded batch

### 2. Agent View — `frontend/app/view/agent/components/ToolBlock.tsx`

**`ToolBlock` component (line 22):**
- **Default collapsed state**: Change initial state in `createAgentAtoms()` (`state.ts` line 51) — new tool nodes should be added to `collapsedNodes` set by default
- Replace click-only toggle with hover+click model:
  - `onMouseEnter` → expand after 150ms delay (unless pinned collapsed)
  - `onMouseLeave` → collapse after 300ms delay (unless pinned open)
  - `onClick` → toggle pinned state
- When collapsed: render only the summary line (line 104-109), append `...` ellipsis
- Remove chevron (`▸`/`▾`) — the `...` ellipsis replaces it as the expand indicator

### 3. Agent View State — `frontend/app/view/agent/state.ts`

**`createToggleNodeCollapsed` action:**
- Add concept of "pinned" nodes (new `pinnedNodes: Set<string>` in DocumentState)
- Toggle click pins/unpins rather than simple expand/collapse

**New tool node insertion logic:**
- When a new `ToolNode` is added to the document, auto-add its ID to `collapsedNodes`
- Exception: if status is `running`, do NOT auto-collapse (add to collapsed only on completion)

### 4. Shared CSS / Tailwind

Add utility classes or a shared component for the collapsed tool line:

```css
.tool-collapsed-line {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    height: 1.75rem;       /* single line height */
    overflow: hidden;
    white-space: nowrap;
    cursor: pointer;
    padding: 0 0.5rem;
    border-radius: 0.25rem;
    transition: background-color 150ms ease;
}

.tool-collapsed-line:hover {
    background-color: rgba(55, 65, 81, 0.5);  /* bg-gray-700/50 */
}

.tool-collapsed-line .tool-ellipsis {
    margin-left: auto;
    color: #6b7280;        /* text-gray-500 */
    font-size: 0.875rem;
}
```

### 5. Transition Animation

Expand/collapse should animate with `max-height` transition (200ms ease) or use `framer-motion`'s `AnimatePresence` if already in the dependency tree. Avoid layout shift — the collapsed line should remain visible at the top during expansion.

## Keyboard Accessibility

- **Esc**: If a tool block is pinned open and has focus, Esc unpins and collapses it
- **Enter/Space**: On focused collapsed tool, toggles pinned expand (same as click)

## Migration Notes

- The Agent View `ToolBlock` already has `collapsed` prop and `onToggle` — this spec extends that model with hover + pin semantics
- The AI Chat Panel `AIToolUse` currently has no collapse concept — this is net-new state
- Both systems should share the collapsed-line rendering logic (extract a `ToolCollapsedLine` component)

## Out of Scope

- Dropdown repositioning (accept edits bar, model selector) — separate spec
- Esc-to-cancel operation bar — separate spec
