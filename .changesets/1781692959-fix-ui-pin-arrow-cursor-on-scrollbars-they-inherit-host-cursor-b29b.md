---
type: patch
---

fix(ui): pin the arrow cursor on scrollbars (they inherit the host cursor)

WebKit `::-webkit-scrollbar*` pseudo-elements inherit the `cursor` of their
scroll-host, so the prior "delete the declaration to get the arrow" approach
left the main agent-pane scrollbar showing the text I-beam (host `cursor: text`)
and the live-tool log scrollbar showing the link hand (host `cursor: pointer`).
Pin `var(--cursor-default)` on the native scrollbar pseudo-elements, replace the
inverted stylelint ban (which forbade the only working fix) with a value-scoped
grep gate (`scripts/check-scrollbar-cursor.sh`), and document the root cause in
docs/retro/retro-scrollbar-cursor-regression-2026-06-17.md.
