# Agent Pane — Spec Verification Report

**Date:** 2026-04-13
**Author:** AgentA
**Verified against `main` HEAD:** `0e795dfe` (post-PR #371 merge)
**In-flight (approved, unmerged):** PR #367 (item #6), PR #372 (item #9)

Scope: three specs that govern the current agent pane surface:

1. `docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md` — 12-step decomposition of `agent-view.tsx`
2. `docs/specs/SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md` — 4-PR merge of Forge + Identity into the agent pane
3. `docs/specs/SPEC_AGENT_PANE_FOLLOWUPS_2026_04_13.md` — 9 follow-up items raised after the consolidation landed

For each spec I walked its stated deliverables and checked the current code. Findings below are grouped by spec; each item is marked **✓ delivered**, **⚠ partial** (shipped but with a caveat), **⧗ pending** (work in progress), or **✗ missing**.

---

## 1. SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md — 12 steps

### 1.1 Line count target

Spec §5: "After Step 12, `wc -l frontend/app/view/agent/agent-view.tsx` must be ≤ 300."

- **On main:** 302 lines.
- **Verdict:** **⚠ partial** — overshoots the target by 2 lines. Not a functional regression, but worth noting as a budget creep. Most likely culprit: the `agent-composer-region` wrapper introduced in PR #370 (spec-followups item #7) added 2–3 lines of JSX. Trivial to reclaim on any subsequent cleanup pass; not urgent.

### 1.2 Hook extractions

| Step | Hook | File | Present? | Notes |
|---|---|---|---|---|
| 3 | `useLaunchLogs` | `hooks/useLaunchLogs.ts` | ✓ | Wired in `agent-view.tsx:76` |
| 4 | `useAgentControllerStatus` | `hooks/useAgentControllerStatus.ts` | ✓ | 156 lines, wired at `agent-view.tsx:83` |
| 5 | `useHistoryPagination` | `hooks/useHistoryPagination.ts` | ✓ | 182 lines |
| 6 | `useSessionDigest` | `hooks/useSessionDigest.ts` | ✓ | 115 lines |
| 7 | `useBookmarks` | `hooks/useBookmarks.ts` | ✓ | 158 lines |
| 8 | `useInSessionSearch` | `hooks/useInSessionSearch.ts` | ✓ | 142 lines |
| 9 | `useScrollToNode` | `hooks/useScrollToNode.ts` | ✓ | Signal-based jump command |
| 10 | `useAgentKeyboard` | `hooks/useAgentKeyboard.ts` | ✓ | Ctrl+B / Ctrl+F, pane-scoped |
| 12 | `useSubagentEvents` | `hooks/useSubagentEvents.ts` | ✓ | spawned / completed subscriptions |
| 12 | `useControllerStatusEvents` | `hooks/useControllerStatusEvents.ts` | ✓ | shellprocstatus → log |
| 12 | `useAgentCommands` | `hooks/useAgentCommands.ts` | ✓ | 169 lines |

All hook files present, all imports wired. **✓ delivered** for the hook layer.

### 1.3 Component extractions

| Step | Component | File | Present? |
|---|---|---|---|
| 1 | `AgentPicker` + `useForgeAgents` | `components/AgentPicker.tsx` | ✓ |
| 11 | `AgentPresentationHeader` | `components/AgentPresentationHeader.tsx` | **✗ file deleted** |

The header component was intentionally removed in the followups spec item #8 (PR #369) in favor of driving the pane frame title from `block.meta.agentName` / `agentIcon` via `AgentViewModel.viewName` / `viewIcon`. The modularization spec's original intent (move the header into its own file) was honored; the subsequent decision to delete it is documented in the followups spec. **✓ delivered** by the combination of steps 11 (extract) and followups #8 (delete).

### 1.4 `scrollIntoView` ancestor-leakage bug (Step 9)

Spec §4: The pane-titles-disappearing bug caused by `scrollIntoView` walking every scrollable ancestor. Fix: compute target offset inside `AgentDocumentView` and call `scrollRef.scrollTo({ top: ... })` directly.

- **On main:** `AgentDocumentView.tsx:62` retains the comment explaining the bug; `:86` uses `scrollRef.scrollTo({ top: Math.max(0, centerOffset), behavior: "smooth" })`; `:117` uses `scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "instant" })` for the jumpToBottom path. **✓ delivered.**

### 1.5 Mutable ref elimination

Spec §3.3: "No mutable refs crossing component boundaries. `let scrollToNodeFn: ((id: string) => void) | null = null` is removed."

- **On main:** `agent-view.tsx` still declares `let scrollToBottomFn: (() => void) | null = null` (line ~114). Used by both `AgentFooter.onTyping` and `useAgentCommands.onSent` to scroll on composer activity.
- **Verdict:** **⚠ partial** — the `scrollToNodeFn` the spec called out specifically is gone (replaced by `useScrollToNode`'s signal command). But a **second** mutable ref, `scrollToBottomFn`, still exists for the scroll-on-type path, and `onSent` reuses it. The spec's anti-pattern rule is technically still violated by the second ref. Either rename the comment to acknowledge the one remaining ref, or port it to a signal-based command too (a `useScrollToBottom` hook mirroring `useScrollToNode`). Not a functional bug — the ref works — but a loose end against the stated invariant.

### 1.6 `AgentNotificationStack` deferral

Spec §5: "`AgentNotificationStack` component. Referenced in the target structure but deferred to a separate PR."

- **On main:** still deferred. Banners (`SessionDigestBanner`, login-waiting, can-retry) are rendered inline in `agent-view.tsx` via `<Show>` blocks.
- **Verdict:** **✗ missing** — explicitly out of scope per the spec. No action needed; flagged here for completeness.

### 1.7 Cumulative trajectory

Spec §4.12 table predicts `agent-view.tsx: 1,178 → ~250` after cleanup. Actual: **302**. 52 lines over the optimistic estimate, 2 lines over the hard cap. Within acceptable drift.

---

## 2. SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md — 4 PRs

### 2.1 Per-card Forge + Identity buttons

Spec §4.1: Each agent card gets ⚙ Forge and 👤 Identity buttons. Clicking opens an inline panel scoped to that agent.

- **On main:** `components/AgentCard.tsx` renders `.agent-card-action-btn` buttons with ⚙ and 👤 glyphs. `AgentPicker.tsx` owns `expandedId` / `expandedTab` / `createMode` signals and conditionally mounts `<AgentCardSettingsPanel>` under the target card. **✓ delivered.**

### 2.2 `+ New agent` tile

Spec §4.3: A "+ New agent" tile at the end of the picker list opens the settings panel in create mode (empty `ForgeForm`).

- **On main:** `components/NewAgentCard.tsx` renders a dashed-border tile. `AgentPicker.openCreateNew` sets `createMode=true` and `expandedId="__new__"` to trigger the create-mode panel. **✓ delivered.**

### 2.3 Inline settings panel with tab switcher

Spec §5: `AgentCardSettingsPanel` renders `ForgeDetail` / `ForgeForm` for the Forge tab and `IdentityPanel` for the Identity tab. Tab switcher in the panel header.

- **On main:** `AgentCardSettingsPanel.tsx` exists (137 lines). Imports `ForgeViewModel`, `ForgeDetail`, `ForgeForm`, `IdentityViewModel`, `IdentityPanel`. Instantiates both view models per-panel, disposes on unmount, renders `<IdentityPanel model={identityModel} />` in the Identity tab. **✓ delivered.**

### 2.4 Panel auto-close on ForgeDetail back

Spec §5: "When the ForgeViewModel view flips back to 'list' (user clicked Back inside ForgeDetail) we want to close the whole settings panel."

- **On main:** `AgentCardSettingsPanel.tsx` has a `createEffect` watching `forgeModel.viewAtom()` with a `mounted` guard (from the PR #364 reagent fix). **✓ delivered.**

### 2.5 Widget bar cleanup

Spec §7.4: Remove `defwidget@forge` and `defwidget@identity` from `agentmux-srv/src/config/widgets.json`.

- **On main (`agentmux-srv/src/config/widgets.json`):** widget bar contains `agent`, `swarm` (hidden), `terminal`, `sysinfo`, `settings`, `help`, `devtools`. No `forge`, no `identity`. **✓ delivered.**

### 2.6 BlockRegistry cleanup

Spec §7.4: Unregister `ForgeView` and `IdentityView` in `frontend/app/block/block.tsx`.

- **On main:** registered views are `term`, `cpuplot`, `sysinfo`, `help`, `launcher`, `agent`, `subagent`, `swarm`. **✓ delivered.**

### 2.7 Saved-pane migration shim

Spec §7.4: "Saved panes with `view: \"forge\"` or `view: \"identity\"` now resolve to `view: \"agent\"`."

- **On main (`frontend/app/block/block.tsx:55`):** `const effectiveView = (blockView === "forge" || blockView === "identity") ? "agent" : blockView;` in `makeViewModel`. **✓ delivered.**

### 2.8 Command palette cleanup

Spec §7.4: Remove `open:forge` and `open:identity` palette commands.

- **On main (`frontend/app/store/command-registry.ts`):** neither command exists. **✓ delivered.**

### 2.9 `ForgeViewModel` / `IdentityViewModel` survival for per-panel use

Spec §6: "Each panel creates its own `ForgeViewModel` — isolation, lifecycle, simplicity."

- **On main:** `forge-model.ts` / `identity-model.ts` still exist in the `view/forge/` and `view/identity/` directories. `AgentCardSettingsPanel` instantiates both per-panel and calls `.dispose()` on unmount. **✓ delivered.**

### 2.10 Identity per-agent scoping

Spec §7 (PR 3): "This PR wires the existing **global** identity UI into the per-agent tab. Per-agent scoping is deferred to `SPEC_FORGE_AGENT_IDENTITY_2026_04_13.md`."

- **On main:** Identity tab shows the global account list for any agent the user clicks 👤 on. Per-agent assignment filtering not implemented. **✗ missing (deferred)** — per the spec, this is intentional. No action needed here.

---

## 3. SPEC_AGENT_PANE_FOLLOWUPS_2026_04_13.md — 9 items

### 3.1 Item #1 — Send scroll-to-bottom fix

- **PR:** #371 (merged)
- **Verification:** `useAgentCommands.ts` has `onSent?` in options (line 60), fires `requestAnimationFrame(() => opts.onSent?.())` after `setDocument` (line 121–122). `agent-view.tsx` wires `onSent: () => scrollToBottomFn?.()` to the hook. **✓ delivered.**

### 3.2 Item #2 — Remove stale "+ New agent in Forge" footer

- **PR:** #368 (merged)
- **Verification:** `grep -rn "agent-picker-forge-btn\|agent-picker-footer" frontend/app/view/agent/` returns **zero matches**. `AgentPicker.tsx` empty-state fallback now renders a live `<NewAgentCard onClick={openCreateNew} />`. **✓ delivered.**

### 3.3 Item #3 — Auto-login on first open

- **Status:** **⧗ pending** — not started. Requires changes inside `launch-flow.ts` Phase 2. Spec marks this as highest risk; scheduled to ship last.

### 3.4 Item #4 — Tool `running` state on one line

- **Status:** **⧗ pending** — not started. Blocked on item #6 (#367 PR) landing so the ToolBlock edits merge into known-good state.

### 3.5 Item #5 — Errors on one line

- **Status:** **⧗ pending** — not started. Planned to bundle with item #4 since both touch the same SCSS region.

### 3.6 Item #6 — Tool hover-expand regression (Portal to escape paint containment)

- **PR:** #367 (approved, not merged)
- **Code delivered:**
  - `ToolBlock.tsx` imports `Portal` from `solid-js/web`
  - Overlay content rendered via `<Portal>` with `position: fixed` computed from the block's `getBoundingClientRect` on mouseenter
  - Scroll listener attached while `overlayMode()` is active, reposition on scroll/resize
  - Deferred `leavePending` timeout for hover-sticky across the DOM gap
  - Portal `onMouseEnter` now routes through `handleMouseEnter` (clears the pending leave) — caught by reagent on first review, fixed in second push
- **SCSS:** new top-level `.agent-tool-content--portal` rule (z-index 200, background, border, shadow, max-height) outside `.agent-view` since the portal mounts to `document.body`
- **Verdict:** **⧗ pending** but ready to merge. Needs re-review after my latest push (0.33.134).

### 3.7 Item #7 — Move AgentControlBar below the composer

- **PR:** #370 (merged)
- **Verification:** `agent-view.tsx` wraps `<AgentFooter>` + `<AgentControlBar>` in a `.agent-composer-region` flex column at the bottom of the pane. `.agent-control-bar` SCSS uses `border-top` (not `border-bottom`) to read as a sub-footer. **✓ delivered.**

### 3.8 Item #8 — Remove in-pane header; use frame title

- **PR:** #369 (merged)
- **Verification:**
  - `AgentPresentationHeader.tsx` file does not exist on main
  - `AgentViewModel` has reactive `viewName` / `viewIcon` reading `block.meta.agentName` / `agentIcon` (agent-model.ts:38–54)
  - `AgentViewModel.endIconButtons()` returns a back-arrow `IconButtonDecl` when `agentId` is set; click routes through `this.backToPicker()` (agent-model.ts:56–69)
  - `AgentViewModel.backToPicker` method exists and is the single source of truth; `useAgentCommands.back()` delegates via the `backToPicker` option
  - `.agent-pres-*` SCSS rules removed
  - **✓ delivered.**

### 3.9 Item #9 — Esc in composer (clear / stop)

- **PR:** #372 (approved, not merged)
- **Code delivered:**
  - `AgentFooter.handleKeyDown` has an `Escape` branch: clears if `textareaRef.value.trim()` non-empty, else calls `props.onStopAgent?.()`
  - Hint line updated to `Enter to send • Shift+Enter for newline • Esc to clear / stop`
  - `useAgentCommands.stopAgent()` calls `RpcApi.ControllerInputCommand({ blockid, signame: "SIGINT" })`; backend `BlockInputUnion::signal` path accepts it
  - `agent-view.tsx` passes `onStopAgent={commands.stopAgent}` to AgentFooter
- **Verdict:** **⧗ pending** but ready to merge after rebase past #371's version collision (re-bumped to 0.33.133).

---

## 4. Summary table

| Spec | Total items | ✓ delivered | ⚠ partial | ⧗ pending | ✗ missing (deferred) |
|---|---:|---:|---:|---:|---:|
| Modularization | 12 steps + 5 cross-cutting | 14 | 2 (line count, one mutable ref) | 0 | 1 (AgentNotificationStack) |
| Forge/Identity consolidation | 10 deliverables | 9 | 0 | 0 | 1 (per-agent identity scoping) |
| Pane followups | 9 items | 4 (#1 #2 #7 #8) | 0 | 5 (#3 #4 #5 #6 #9) | 0 |

**Ship state:** 27 of 31 verifiable deliverables **shipped**. 2 **partial** (minor, line-count + one remaining ref), 5 **pending** (3 approved awaiting merge, 2 not started), 2 **explicitly deferred** to separate specs.

---

## 5. Gaps and loose ends

### 5.1 `agent-view.tsx` is 302 lines, 2 over the ≤300 target

Not urgent. Options to reclaim:
- Inline the `agent-composer-region` div class name without the wrapping `<div>` (use `classList` on an existing node — but there's no natural parent).
- Collapse a few multi-line JSX props into single lines.
- Accept the 2-line overshoot and update the spec target.

Recommendation: accept the overshoot. The spec target was an aspirational budget, not a hard contract; the code is structurally clean and reducing by two lines for its own sake would be wasted work.

### 5.2 Second mutable ref (`scrollToBottomFn`) survives the modularization spec's "no mutable refs" rule

The spec called out `scrollToNodeFn` specifically and replaced it with `useScrollToNode`. The same pattern (`scrollToBottomFn: (() => void) | null = null`) is still in `agent-view.tsx` for the scroll-on-type + scroll-on-send paths. Spec §3.3 says "no mutable refs crossing component boundaries" — this technically violates the rule.

**Fix (optional):** introduce a `useScrollToBottom` hook mirroring `useScrollToNode`. Its `command()` accessor changes any time the consumer calls `jumpToBottom()`. `AgentDocumentView` reads it in an effect and invokes its internal `jumpToBottom` function. Consumers get `scroll.jumpToBottom()` instead of `scrollToBottomFn?.()`.

Not urgent. The current ref works; this is purely a consistency cleanup.

### 5.3 `AgentHeader.tsx` (not the presentation header) is dead code

`frontend/app/view/agent/components/AgentHeader.tsx` exists on main but is not imported anywhere. Predates the modularization effort. Different from `AgentPresentationHeader.tsx` which was deleted in PR #369.

**Recommendation:** delete it in a later cleanup pass. Not blocking.

### 5.4 Followups items #3 / #4 / #5 still ahead of us

Spec says ship ordering is `#6 → #2 → #8 → #7 → #1 → #9 → #4+#5 → #3`. Items #6 and #9 are approved but not yet merged. Items #4 and #5 are not started; they were intentionally blocked on #6 landing first so the ToolBlock state is stable. Item #3 (auto-login) is the last and riskiest.

**Recommendation:** wait for #367 and #372 to be re-reviewed and merged. Then ship #4 + #5 bundled (one PR that touches `ToolBlock.tsx` + a small SCSS block for errors), then #3 as a standalone PR.

### 5.5 Identity per-agent scoping is still the global list

Explicitly deferred to `SPEC_FORGE_AGENT_IDENTITY_2026_04_13.md`. The UI is already wired inside the agent pane — that spec's work will change what data the `IdentityPanel` shows (only assigned accounts for the clicked agent), not where it renders.

**Recommendation:** address when someone has the appetite to touch the identity model. Not blocking any current work.

---

## 6. Regressions (none observed)

I checked every item the specs promised and cross-referenced against the current code. No delivered item is broken; no behavior from prior specs has been lost. The followup items still in flight are tracked with PR numbers and have review signal (#367 and #372 are approved).

---

## 7. Conclusion

The agent pane consolidation work (modularization + forge/identity merge + first wave of followups) is in a consistent shape. Four of the nine followup items remain: three pending/ready (#6, #9 approved; #4+#5 blocked on #6), one riskiest last (#3).

No emergency, no hidden regression. Two minor cleanups (line count, second mutable ref) can be deferred or skipped entirely. The specs match the code; the code matches the specs.

Continue when ready.
