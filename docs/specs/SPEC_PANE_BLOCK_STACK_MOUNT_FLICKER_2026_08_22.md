# Pane block-stack mount flicker — root causes + reveal-gate generalization

**Status:** Phases 1-4 implemented (see §6)
**Owner:** AgentX
**Date:** 2026-08-22
**Scope:** `frontend/layout/lib/layoutStack.ts`, `layoutNodeModels.ts`'s `activeKeyFor`,
`frontend/layout/lib/tilelayout-shared.tsx`, `frontend/app/block/block.tsx`,
`frontend/app/store/tab-reveal.ts`, `frontend/app/element/PaneTabStrip.*`,
`frontend/app/view/agent/agent-view.tsx` (`handleNewAgentTab`), `quick-fork.ts`
**Related:** `SPEC_TAB_CONTENT_REVEAL_GATE.md` (the whole-tab-switch analog this
spec generalizes — read that one first, this spec assumes its mechanism),
`REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md` (a different flicker
class — scroll-pin lag, not remount), `SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md`
(introduced the block-stack mechanism itself), `SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`
(the strip's shrink-to-fit sizing this spec's §3 touches)

**Driving observation:** the repo owner reported visible flashing "even when
clicking the + to get a new agent" — i.e. inside a single pane, adding a
sibling to that pane's own tab strip (Agent History, quick-fork, and the "+"
new-agent-tab button all go through the same mechanism), not just on
whole-window tab switches. `SPEC_TAB_CONTENT_REVEAL_GATE.md` already solved
the analogous whole-tab-switch flicker; this class was out of that spec's
scope by its own admission (§"Out of scope": "Backend-driven switches...
bypass `setActiveTab` and don't currently fire the gate. They're rare;
address... if it becomes a pain point." — it has).

---

## 1. Symptom

Click the "+" on a pane's own tab strip (or trigger Quick Fork, or open Agent
History) and the pane visibly flashes through several uncoordinated states
before settling:

1. A new pill pops into the tab strip — the strip itself snaps from its
   hidden/single-tab width to its multi-tab width with no transition.
2. The pane's content area hard-cuts to a bare `BrainSpinner` (block.tsx's
   `ready()` gate is false — the new block's `WaveObj` hasn't round-tripped
   from the backend yet).
3. Once the block's data arrives, it hard-cuts again to `AgentPicker` (a
   freshly-created block has no `agentId` meta yet — `agent-view.tsx`'s
   `<Show when={agentId()} fallback={<AgentPicker/>}>`).
4. If this is a quick-fork or a launch-in-place, a THIRD hard cut follows
   once the launch completes and the real `AgentPresentationView` mounts.

Each stage is a separate, uncoordinated paint — exactly the "user perceives
each transition as flicker" pattern `SPEC_TAB_CONTENT_REVEAL_GATE.md` already
diagnosed for whole-tab switches, just one level down, inside a single pane.

## 2. Root causes (four distinct, independently-fixable sources)

### 2.1 Every block-stack push/switch forces a full subtree remount — architectural, by design

`activeKeyFor(node)` (`layoutNodeModels.ts:142-144`) returns
`` `${node.id}:${node.data.activeBlockId}` `` once a leaf has a stack. The
tile renderer's `<Key each={leafs()} by={activeKeyFor}>` (`tilelayout-shared.tsx:173`,
shared by all three platform `TileLayout.*.tsx` variants) tears down and
reconstructs the whole leaf subtree whenever that key changes.
`pushBlockOntoStack`/`setActiveBlockInStack` (`layoutStack.ts:52-92`) change
`activeBlockId` and explicitly evict the cached `NodeModel`
(`model.nodeModels.delete(nodeId)`, `layoutStack.ts:64`) — both deliberately,
per that file's own header comment: `NodeModel.blockId` is captured once at
construction, so a remount is "correct" here, not a bug, mirroring how every
other blockId transition in this codebase already works.

**This is not a bug to patch** — it's the documented mechanism. But it does
mean every "+", Quick Fork, or Agent History click is, underneath, a full
component-tree teardown+rebuild, and nothing today hides that transition from
the user (contrast: `NodeModel`/`ViewModel` for a *different, already-visible*
pane never remounts on unrelated activity).

### 2.2 No reveal gate covers pane-local (leaf-scoped) mounts — the real gap

`tab-reveal.ts`'s gate is real, tested, and already fixes the *identical*
flicker class for whole-tab switches (`SPEC_TAB_CONTENT_REVEAL_GATE.md`):
`holdRevealGate()`/`scheduleRevealLift()` set one global `tabSwitching`
signal; `workspace.tsx:66-69` reads it and applies
`visibility:hidden`/`opacity:0` to the tab-content root that matches
`tid === tabId()`. This paints the whole tab atomically once a window of
Long-Task-free frames (or an 800ms hard cap) has passed, instead of the
piecemeal cascade.

It is **scoped to the whole active tab**, wired only from `setActiveTab`/
`createTab` (`tab-actions.ts:17,33,95`). `pushBlockOntoStack` never calls
either function — confirmed by the spec's own scope note quoted above. A
block-stack push/switch happens **inside an already-visible tab**, so hiding
the *entire* tab (the existing mechanism's only granularity) would be wrong
even if it were wired in — it would blank out sibling panes that aren't
remounting at all. **Generalizing this from "one global boolean gating the
whole tab" to "a per-leaf gate" is the architectural change this spec
proposes** (§4).

### 2.3 Three uncoordinated hard cuts inside the gate window, not one

Even with a gate in place, what's *inside* the hidden window today is itself
three separate mount stages (§1's steps 2-4), each synchronous and
un-cross-faded: `block.tsx:294,304`'s `ready()` gate falls back to a bare
`<BrainSpinner/>` with no fade class wired (contrast the fade support
`BrainSpinner.scss` already has, used only by `AgentPresentationView`'s own
separate loading overlay, `agent-view.tsx:805-862` — never by the generic
`Block` gate). A reveal gate hides the *transition*, but the frame that gets
revealed at the end should be the *final* state, not whichever of the three
stages happened to be current when the settle window elapsed — so this needs
fixing regardless of §2.2, or the gate will sometimes reveal a still-loading
`BrainSpinner`/`AgentPicker` frame.

### 2.4 CSS layout-shift when the strip itself appears — small, independent

`.pane-tab-strip` (`PaneTabStrip.scss:11-27`) has no `transition` on width.
`visibleTabs()` (`agent-view.tsx:265`) jumps from `[]` to the full tab list
the instant a 2nd stack member exists, snapping the shrink-to-fit strip's
width instantly. Because the strip floats over content instead of reserving
its own row (`agent-view.tsx:424-436`, `SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md`),
this sudden widening can occlude previously-visible pane chrome with no
transition softening it. Independent of §2.1-2.3; fixable on its own.

## 3. Non-causes checked and ruled out

- **`markBlockRecentlyCreated`** (`layoutPersistence.ts:84-100`) age-gates
  `pruneDanglingLeaves` against the real gap between a leaf landing in the
  local tree and `tab.blockids` catching up from the backend — a correctness
  guard, not a paint source itself. It's evidence of the same "local state
  mutates before backend data exists" shape as §2.3, not a separate bug.
- **CEF's pane-pool pre-warming** (`agentmux-cef/src/floating_pane.rs:391-515`)
  solves flicker for real OS-level `WS_POPUP` + CEF-child windows (browser
  panes only). Agent blocks are pure Solid/DOM content inside the existing
  webview — there's no equivalent "pre-warmed hidden instance" concept for a
  `NodeModel`/`ViewModel`, and building one is a much larger lift than §4's
  proposal for the same visual win. Not recommended (see §5, rejected
  alternative).

## 4. Design space

### Option A — Generalize the reveal gate to leaf scope (recommended)

Replace `tab-reveal.ts`'s single global `tabSwitching` boolean with a small
`Set<string>` of currently-gating **node ids** (not tab ids). `holdRevealGate`/
`scheduleRevealLift` take an optional `nodeId` — omitted, they behave exactly
as today (whole-tab gate, for `setActiveTab`/`createTab`); passed, they gate
only that one leaf.

- `pushBlockOntoStack`/`setActiveBlockInStack` (`layoutStack.ts`) call
  `holdRevealGate(nodeId)` before mutating, and the caller (`handleNewAgentTab`,
  `openOrFocusHistoryTab`, `quickForkAgent`) calls `scheduleRevealLift(nodeId)`
  once its own async work (RPC round-trip, launch) resolves — same
  hold-then-schedule pairing `tab-actions.ts` already establishes for the
  whole-tab case.
- The tile renderer's leaf wrapper (wherever `DisplayNode`/the shared
  `tilelayout-shared.tsx` renders each leaf) reads
  `gatingNodeIds().has(node.id)` and applies the identical
  `visibility:hidden`/`opacity:0` treatment `workspace.tsx` uses today, scoped
  to just that leaf's DOM subtree instead of the whole tab-content root.
- Detector logic (Long-Task-free settle window, `MAX_GATE_MS` hard cap)
  carries over unchanged — it's already generic over "when did the last long
  task fire," not tab-specific.

**This is the architectural piece** — today's mechanism has no concept of
"more than one thing can be settling at once, at different scopes." A `Set`
instead of a `boolean`, and a per-leaf reader instead of a single tab-root
reader, is the minimal change that adds that concept without touching the
detector algorithm at all.

### Option B — Per-block skeleton / fade only (smaller, incremental)

Fix §2.3 in isolation: wire `BrainSpinner`'s existing fade support into the
generic `Block` gate, and cross-fade the `AgentPicker`→`AgentPresentationView`
hard cut. Reduces the *number* of jarring cuts without hiding the transition
window itself — the reveal-gate's core promise ("atomic before/after") isn't
met, but it's a same-day fix with no architectural change.

**Not sufficient alone** — `SPEC_TAB_CONTENT_REVEAL_GATE.md` already tried
"just fade the pieces" implicitly (the pre-gate baseline) and it wasn't
enough to read as non-janky; that's *why* the gate was built. No reason to
expect a different outcome one level down.

### Option C — Pre-warmed hidden `NodeModel` pool (rejected)

Mirror the CEF pane-pool: pre-construct a hidden, off-screen `NodeModel`/
`ViewModel` speculatively, promote it into place instead of remounting fresh.
Would eliminate the remount entirely rather than just hiding it — but no
generic "pre-warm an arbitrary block type" concept exists in the Solid layer
today (unlike CEF's browser-specific HWND pool), and speculatively spawning
agent controllers ahead of a click has real cost/side-effect implications
(a `pane.open` + controller creation isn't free or side-effect-free the way
warming an empty browser window is). Rejected for this pass — Option A gets
the same *perceived* result (no visible flicker) without inventing new
runtime infrastructure.

## 5. Decision

**Option A (generalize the reveal gate) + Option B (fix the fallback cuts
inside the window) together.** Neither alone is sufficient: A hides the
transition but reveals whichever of the three uncoordinated stages happens to
be current when it lifts (still occasionally wrong) unless B also ensures the
revealed frame is the *final* one; B alone doesn't hide the transition at all.
C is rejected per §4.

§2.4 (CSS layout-shift) is a small, independent fix — do it regardless of A/B,
it costs nothing and helps even mid-transition frames look less jarring.

## 6. Phasing

| Phase | Deliverable | Risk | Notes |
|---|---|---|---|
| **1** ✅ | Generalize `tab-reveal.ts`: a leaf-scoped `gatingNodeIds()` `Set<string>` alongside (not replacing) the existing `tabSwitching` boolean, plus new `holdLeafRevealGate(nodeId)`/`scheduleLeafRevealLift(nodeId)` exports. **Implementation deviated from this row's original wording in one way**: rather than adding an optional `nodeId` param to the existing `holdRevealGate`/`scheduleRevealLift`, the leaf gate got its own separate functions — lower risk (zero chance of a shared-state bug touching the already-shipped whole-tab gate) and the leaf gate's detector reuses the already-existing, independently-cancellable `@/app/util/settle-detector`'s `scheduleOnSettle` (its own doc comment had already anticipated this exact "N independent panes" need) instead of duplicating the whole-tab gate's hand-rolled detector a second time. | low | Pure addition; `tab-reveal.test.ts` extended with 7 new tests for the leaf gate (independence between two node ids, independence from the whole-tab gate) — all existing whole-tab tests pass unchanged. |
| **2** ✅ (2 real races fixed during PR #2761's review) | Wired the leaf-scoped gate into every caller that forces this remount: `handleNewAgentTab` + `handleTabSwitch` (`agent-view.tsx`, both the create-new AND switch-between-already-open-pills paths — Codex's review caught that the original PR only gated the former), `openOrFocusHistoryTab` (both its create-new and switch-to-existing paths), `quickForkAgent`, and `handleTermTabAdd` + `handleTermTabSwitch` (`term.tsx` — the terminal-pane analog of the same two agent-pane gaps). Tile renderer reads `gatingNodeIds()` per leaf, merged into the existing `tileTransform()` style object (not a wrapper element, to leave each leaf's absolute positioning untouched) in all three platform `TileLayout.*.tsx` files identically. **`holdLeafRevealGate`/`scheduleLeafRevealLift` gained a generation-token pairing** (`holdLeafRevealGate` returns a token, `scheduleLeafRevealLift` takes it back) after Codex's review found two real races in the original token-less version: (a) two overlapping operations on the same node id (e.g. two rapid "+" clicks) — the OLDER operation's completion could reveal the pane while the NEWER one was still in flight; (b) a single slow operation whose own hold safety-net timeout fires before it finishes, then the later schedule call re-hides the already-revealed pane, producing a visible→hidden→visible flash. A call is now a no-op whenever its generation is stale — superseded by a newer hold, or already resolved once. **A third reagent review round then caught the close path**: `handleTabClose` (`agent-view.tsx`) and `handleTermTabClose` (`term.tsx`) both called `closeBlockInStack` directly, ungated — closing the *active* member of a multi-member stack reassigns `activeBlockId` and evicts the `NodeModel` exactly like a switch does, so closing tab 2 of 3 still flickered. Both now hold the gate before the call and schedule the lift in a `.finally()`, matching the switch-handler pattern — but **only when the closed tab is the resolved node's own active member** (`node.data?.activeBlockId === targetBlockId`). A same-round reagent finding caught that gating unconditionally was itself a regression: `gatingNodeIds()` hides a leaf's entire rendered content regardless of whether a remount is actually about to happen, so closing a *background* (non-active) tab — which `closeBlockInStack` handles as a pure stack-array edit with no `activeBlockId` reassignment — was being hidden and re-revealed for no reason, a new flicker on a path that was previously flicker-free. The active-member check mirrors the guard `handleTabSwitch`/`handleTermTabSwitch` already had (`if (targetBlockId === activeBlockId()) return;`). `clearLeafRevealGate` (wired into `closeNode` in Phase 2) already handles the last-member-closes-the-pane case, so no separate handling was needed there. | med | Mitigated the "leaf gated forever" risk via the try/finally pairing (mirrors `tab-actions.ts`'s own established pattern) — every caller's finally block covers all its exit paths. The two races above are covered by dedicated regression tests in `tab-reveal.test.ts`. |
| **3** ✅ | Fixed the fallback-stage hard cuts (§2.3). **`Block`'s `ready()` gate** (`block.tsx`): the fallback `BrainSpinner` now stays mounted, absolutely positioned atop the real content (`.block-ready-gate-host.is-overlay`, generation-free since only one transition — false→true — is possible per mount), and cross-fades out over 200ms instead of an instant `<Show>` swap; unmounts once the fade completes. Required making `.tile-leaf` (`tilelayout.scss`) a positioned containing block, so the overlay's `inset:0` resolves against its own (gap-padded, in split layouts) box instead of skipping past it to `.tile-node`. **`AgentPicker`→`AgentPresentationView`** (`agent-view.tsx`): same technique, agent-pane-specific — `pickerVisible`/`pickerFadingOut` state keeps `AgentPicker` mounted (`.agent-picker-host.is-overlay`) fading out on top of the now-mounting `AgentPresentationView` (which shows its own already-existing loading-overlay/history-load fade underneath, so the net effect is picker → spinner → content with no hard cuts) instead of unmounting it the instant `agentId()` is set. Both use the established "apply the overlay class the SAME render the transition starts, defer the actual opacity-fade trigger one `requestAnimationFrame` so there's a real frame to fade FROM" pattern. | low | UI polish, no state-machine change. Not covered by new automated tests — `agent-view.tsx`/`block.tsx` have no existing component-render test harness (consistent with the rest of this codebase's testing approach for these two files); verified via `tsc --noEmit`, the full `vitest run` suite (no regressions), and manual reasoning about the CSS containing-block chain. Not manually verified in a running browser — see PR description. |
| **4** ✅ | `.pane-tab-strip` width transition (§2.4). | low | **Implementation deviated from this row's original wording**: a first attempt used pure CSS (`interpolate-size: allow-keywords` + `transition: width`), but Codex's review of PR #2768 caught that this never actually animates — `width` stays `auto` the whole time here (only the DOM-content-driven USED size changes when a tab is added/removed), and CSS transitions only fire on a discrete SPECIFIED-value change. Replaced with a measured FLIP transition in `PaneTabStrip.tsx` (hold the old measured width, force a reflow, transition to the new one, clear back to `auto` after) — no CSS feature detection needed either way. Gated behind a new opt-in `animateWidth` prop, passed only by `term.tsx`/`editor-tab-strip.tsx` (the two consumers that stay genuinely shrink-to-fit) — Codex also caught that the agent pane overrides the strip to a fixed `left:0;right:0` full-width box (`SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md`), where an explicit inline `width` would fight that override. | 5 new tests in `PaneTabStrip.test.tsx` cover: no-op when `animateWidth` is unset, the hold→transition sequence, cleanup after the duration elapses, no-op when the measured width doesn't change, and no animation on the initial mount. |

Phase 1-2 together deliver the actual fix (no more piecemeal-paint flicker on
"+"/Quick Fork/Agent History); Phase 3 makes sure what gets revealed is
correct (no more hard cuts among BrainSpinner/AgentPicker/AgentPresentationView
inside that revealed frame either); Phase 4 is independent polish, landed
alongside the others in the same pass.

## 7. Open questions

- **Nested gating**: can a leaf-scoped gate and the whole-tab gate ever be
  active at once (e.g. quick-fork triggered mid-tab-switch)? Likely rare
  enough to not special-case — the leaf gate is a strict subset of "hidden,"
  so a doubly-gated leaf is still just hidden, no visible conflict. Confirm
  with a test once Phase 2 lands.
- **Settle-window tuning at leaf scope**: `SETTLE_MS`/`LONG_TASK_THRESHOLD_MS`
  were tuned for whole-tab-switch cost. A single leaf's mount is cheaper —
  worth re-measuring whether the same 80ms/50ms constants still make sense,
  or whether a leaf-scoped gate should use a shorter settle window (§6 Phase
  2 can carry a follow-up measurement task).
- **Quick-fork's own launch RPC chain** (`quick-fork.ts`) is the longest
  async path of the three callers (fork → identity lookup → pane.open →
  launch, several RPCs deep). Confirm the `MAX_GATE_MS` hard cap (800ms) is
  generous enough not to reveal mid-launch on a slow backend — may need a
  per-call override rather than one constant shared with instant
  Agent-History opens.

## 8. Non-goals (this pass)

- Whole-tab reveal-gate behavior (`SPEC_TAB_CONTENT_REVEAL_GATE.md`) is
  unchanged — this spec only adds a second, narrower granularity alongside it.
- Not attempting Option C's pre-warm pool (§4) — flagged as a possible
  future direction if Option A's perceived latency (however small) ever
  becomes a complaint on its own.
- Not touching the CEF browser-pane pool (`floating_pane.rs`) — unrelated
  code path, already solves its own (different) flicker class.
