<!-- Captured from GitHub issue #782 — agentmuxai/agentmux -->

## Why

PR #773's virtualization retrofit was reverted (PR #781) after it caused gaps, broken tables, and streaming cut-off. The pattern was clear: each fix-as-we-go workaround introduced new edges because virtualization was bolted onto code that assumed every node was in the DOM.

## Plan

Architectural redesign in `docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md` on branch `agenta/spec-agent-pane-virt-redesign`. Treats virtualization as a first-class concern + adds intelligent perf probing built into the render contract.

## Key design points

- **Per-kind size estimators** instead of one global `estimateSize: 80` (eliminates gaps for short messages)
- **Hybrid render**: last 50 nodes unvirtualized (streaming buffer) — eliminates streaming-cut-off bug class entirely
- **Scroll state in store, not DOM** (`headAnchor` + `stickToBottom`) — single source of truth
- **`table-layout: fixed` + `width: 100%` rows** — fixes table distortion
- **CSS `overflow-anchor: auto`** as Chromium-native belt to JS anchor's suspenders
- **Integrated perf probing** — per-kind marks (p50/p95/max), estimator-miss detection, layout-shift attribution scoped to agent pane, dev HUD extension to slice #9 diag panel

## Industry references

Validated against: `react-virtuoso`'s Message List, Stream's `VirtualizedMessageList`, `use-stick-to-bottom`, Discord's mobile rewrite, `react-virtualized` Table patterns, `overflow-anchor` MDN guidance.

## Effort

~4.5 days across 4 phases (foundation, virtualization layer, perf probing, hardening).

## Cross-references

- PR #781 — revert of #773 (must merge before this work starts)
- Issue #774 — tab content reveal gate (complementary)
- `frontend/perf/marks.ts`, `frontend/app/devtools/diag-panel.tsx` — extension points

🤖 Generated with [Claude Code](https://claude.com/claude-code)
