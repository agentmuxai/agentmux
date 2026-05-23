# Retro: Agent-pane cascade → replaceChild → blank-tab content area

**Date:** 2026-05-23
**Author:** AgentA
**Severity:** High — user-visible blank tab content with red error text; no in-app recovery

---

## 1. Symptom

After a v0.33.900-line portable launched, a SolidJS reactive cascade in a single
agent pane triggered a `NotFoundError: The node to be removed is not a child of
this node` from Solid's reconciler. Only the workspace-level `<ErrorBoundary>`
caught it, so the ENTIRE tab content area went blank with red error text. The
status bar and tab bar remained alive, but the user could not recover
individual panes — they had to close the whole tab and re-open from scratch,
losing any state in other (perfectly healthy) panes that lived in the same
tab.

## 2. Why one pane took down the whole tab

The render tree at the time:

```
WorkspaceRoot
  <ErrorBoundary fallback={...}>     ← only boundary above per-block roots
    TabContent
      Layout
        BlockNode (agent pane)       ← cascade source
        BlockNode (terminal pane)
        BlockNode (browser pane)
```

When the agent-pane cascade threw, Solid unwound everything up to the nearest
boundary — which was the workspace's. The boundary's fallback replaced the
whole subtree, so the layout + every sibling block (which were not broken) all
disappeared.

The `BlockFrame_Default_Component` (`frontend/app/block/blockframe.tsx`) had
two narrow `<ErrorBoundary>` instances inside it — one wrapping the title-bar
`headerElem` and one wrapping the view's child `Suspense`-fallback content
inside `block.tsx`'s `BlockFull`/`BlockSubBlock`. Both of those caught errors
inside their own narrow children but NEITHER guarded the BlockFrame's outer
chrome (the `.block-frame-default` wrapper, agent-color memo, focus-effect
chain, ConnStatusOverlay, BlockMask) — so a throw originating from any of
those, OR from a cascading reactive write that cleaned up a hook owner before
the next dispatch landed, escaped past both and bubbled to the workspace
boundary.

## 3. The cascade source (covered by separate PRs)

The exact root cause was the cascade-during-dispatch class documented in
`docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md` and partially fixed in
PR #878 (soft-dispatch variant for async sites). The remaining work to fully
prevent the cascade is tracked separately as cascade follow-ups PR-3 + PR-4.

This retro is about the OTHER axis: even after we eliminate the cascade
source, an unforeseen renderer fault in one pane must not blank the whole tab.

## 4. Follow-ups (cascade-follow-up series)

| # | Follow-up | Status |
|---|---|---|
| 1 | Per-block `<ErrorBoundary>` + localized reload UI | **this PR** |
| 2 | Recover conversation history when a pane reloads (use the recent-sessions cascade restore path) | not started |
| 3 | Eliminate the cascade source (cleanup-ordering refactor in agent-pane-state-store) | not started |
| 4 | Single-slot per-pane registration helper (the architectural option from §6.2 of the LIFECYCLE_DISPATCH_LEAK analysis) | not started |

## 5. Design — per-block error boundary

Wrap the `.block-frame-default` body in its own `<ErrorBoundary>`. The fallback:

- Logs the catch via `fe_log_structured` (`level: "error"`, `module: "block-error-boundary"`),
  with `block_id`, `view_type`, `error_name`, `error_message`, and `error_stack`.
  This gives every future cascade a server-side trace next to the existing
  uncaught-error forwarder.
- Renders a localized panel: red border, error icon, headline, the error
  message, and a collapsible stack section.
- Offers two actions:
  - **Reload pane** — calls Solid's `reset` so the boundary re-mounts the
    children fresh. Reactive owners are torn down and recreated. Any in-flight
    reactive state in the broken pane is discarded.
  - **Close pane** — destroys the block via the existing `nodeModel.onClose`
    affordance.
- Reads ONLY from the props the boundary passes in (`blockId`, `viewType`,
  `error`, `reset`, `onClose`). Does NOT touch the agent-pane-state-store,
  the document atom, or any reactive primitive that the broken pane was
  using — that graph may be half-flushed.

## 6. Defense in depth

The existing workspace-level boundary stays put. Two boundaries (per-block +
workspace) means:

- If the per-block fallback itself somehow throws, the workspace catches.
- If something inside the workspace layout chrome (not inside a block) throws,
  the workspace still catches.

We are not relying on the per-block boundary to be flawless — it is the FIRST
line of defense, the workspace is the safety net.

---

🤖 Authored by AgentA, 2026-05-23. Cascade follow-up 1 of 4.
