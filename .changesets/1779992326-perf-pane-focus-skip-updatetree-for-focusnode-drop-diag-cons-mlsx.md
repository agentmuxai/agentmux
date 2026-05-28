---
type: patch
---

perf(pane-focus): skip updateTree for FocusNode + drop diag console.logs

Two small, all-cross-platform wins on the click → focused-border-paint
chain (issue #1136, full analysis in
`docs/analyses/ANALYSIS_PANE_FOCUS_PAINT_LATENCY_2026-05-28.md`):

- **Skip `updateTree()` for `FocusNode` actions**
  (`frontend/layout/lib/layoutModel.ts`). The reducer previously ran
  a full rebalance + per-leaf transform recompute after every action.
  `FocusNode` only mutates `treeState.focusedNodeId` — topology, sizes,
  and per-leaf transforms are unchanged. The reactive `isFocused` memos
  still get notified via the `localTreeStateAtom._set` immediately after.
  Savings scale with #panes-per-tab; the dominant synchronous cost on
  the click path is gone.
- **Drop the two diagnostic `console.log`s in `handleChildFocus`**
  (`frontend/app/block/block.tsx`). `getElemAsStr(event.target)` walked
  the DOM on every focusin event; the unused `getElemAsStr` import is
  removed too.

No platform `cfg` / `*.linux.tsx` gates; benefits every platform
uniformly.
