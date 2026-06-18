# Retro: Agent Pane Copy/Paste Regression

**Date:** 2026-03-17
**Severity:** Medium — core UX broken in agent pane
**Introduced:** PR #148 (commit `8d3e867`, 2026-03-16)
**Author:** AgentA (Claude Code)
**Fixed:** This retro (2026-03-17)

---

## What happened

Copy/paste stopped working in the agent pane. Users could not select text in status logs, raw output, or user messages. The context menu "Copy" action was dead because no text selection could be created.

## Root cause

PR #148 added `user-select: none; cursor: default;` to the root `.agent-view` element to prevent accidental selection during drag/resize interactions. Overrides (`user-select: text; cursor: text;`) were added to `.agent-markdown-block` and `.agent-tool-content`, but **three content areas were missed**:

| Selector | Content | Override? |
|----------|---------|-----------|
| `.agent-document` | Main scrollable content area | Missing |
| `.agent-status-log` | Terminal-style status lines | Missing |
| `.agent-raw-output` | Pre-formatted CLI output | Covered by parent now |
| `.agent-user-message` | User's input messages | Missing |
| `.agent-markdown-block` | Agent markdown responses | Had override |
| `.agent-tool-content` | Tool execution results | Had override |

## Fix applied

Added `user-select: text; cursor: text;` to:
- `.agent-document` (covers all document content including raw output)
- `.agent-status-log`
- `.agent-user-message`

The root `user-select: none` is kept because the pane header, picker, and chrome elements should not be selectable.

## Other regressions checked

### From PR #151 (UI density tightening, 2026-03-17)

| Area | Status | Notes |
|------|--------|-------|
| Tool block collapse/expand | OK | `onClick` on `.agent-tool-block` still fires, padding reduction doesn't affect click targets |
| Tool block content selection | OK | `.agent-tool-content` has `e.stopPropagation()` + `user-select: text` |
| Forge card click targets | OK | 6px 8px padding is still sufficient for mouse targets |
| Forge form inputs | OK | Form elements have their own padding, unaffected by container reduction |
| Markdown code blocks | OK | `padding: 1px 4px` inline code unchanged |
| Thinking blocks | OK | Border-left + padding-left still renders correctly at 12px |
| Agent picker | N/A | Has its own padding (16px), was not changed |

### From PR #152 (uptime clock sync)

| Area | Status | Notes |
|------|--------|-------|
| Status bar uptime | OK | Driven by sysinfo events now, no setInterval drift |
| libc dependency | OK | Properly gated with `cfg(unix)`, Windows unaffected |

### From PR #153 (health monitoring)

| Area | Status | Notes |
|------|--------|-------|
| Subprocess performance | OK | Health classification adds negligible overhead per NDJSON line |
| Watchdog timer | OK | 5s interval only runs during active turns, self-terminates |
| Double JSON parse | Minor | `stdout_reader` now parses JSON twice (once for health, once for session_id). Low priority but could be unified in a follow-up |

## Lessons

1. **When adding `user-select: none` to a container, audit ALL child content areas.** It's easy to add overrides to the two you're looking at and miss three others.

2. **Copy/paste is a core UX feature that should have a smoke test.** Consider adding a manual test checklist: "can select text in status log, markdown, tool output, user messages."

3. **CSS cascade bugs are invisible in code review.** The diff for PR #148 looked correct — `user-select: none` on root, overrides on content. The missing overrides are only visible if you mentally enumerate all content-bearing child selectors.

## Action items

- [x] Fix: add `user-select: text` to `.agent-document`, `.agent-status-log`, `.agent-user-message`
- [ ] Follow-up: unify double JSON parse in `stdout_reader` (health + session_id)
- [ ] Consider: add "can select text" to agent pane manual test checklist
