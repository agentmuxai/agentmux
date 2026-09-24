# PLAN: Universal Pane Tabs — Implementation Task Breakdown

**Date:** 2026-09-17
**Status:** active — PRs touching this work, newest first: #3309
**Companion (design):** `docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md`
— read that first; this doc is the task-level execution plan for it, not a
restatement of the design.
**Branch policy:** implemented on a dedicated long-lived branch
(`agento/pane-tabs-universal`), NOT merged on approval. This redesign is
far-reaching enough (touches chrome for every widget type eventually, a new
shared component, backend RPC additions, keyboard bindings) that the repo owner
wants it tested rigorously locally before any merge decision — every task below
gets verified via typecheck + unit tests + a real `task dev` manual pass, but the
PR (once opened, for CI signal and visibility) stays unmerged until the repo owner
explicitly approves after their own testing.
**Sequencing note:** only Task Group A (groundwork) and Task Group B (agent/term
migration onto the new unified chrome) are committed to in this pass. Extending to
browser/editor/sysinfo/etc. (the design spec's §5 Phase 1/2/3/4) is real, separable
follow-up work — attempting all of it in one pass would produce an unreviewable,
untestable diff. Task Group C below scopes exactly how far this pass goes and what
it deliberately leaves for a follow-up plan.

---

## Task Group A — Groundwork (no visible behavior change yet)

### A1. `PaneHeaderTabStrip` component
**New file:** `frontend/app/element/PaneHeaderTabStrip.tsx`.
Supersedes the combination of `BlockFrame_Header` + `PaneTabStrip` for hoisted
panes (design spec §4.1). Renders, left to right: optional leading Pane icon, the
tab pills (reusing `PaneTabStrip`'s existing pill rendering — do not reinvent tab
pill markup, extract/reuse), the "+" button (wired in A3), and the pane-level
control cluster (minimize/magnify/close-Pane, moved verbatim from
`BlockFrame_Header`'s `OptMinimizeButton`/`OptMagnifyButton`/close-decl,
`blockframe.tsx:168-253`).
**Test:** a focused unit test file (`PaneHeaderTabStrip.test.tsx`) covering: single
tab renders one pill + control cluster (no second row); N tabs render N pills;
clicking a tab pill fires the same `onActivate` contract `PaneTabStrip` already has;
control cluster buttons fire the same callbacks `BlockFrame_Header`'s did (so this
is provably a drop-in replacement, not a new behavior).

### A2. `pane:tabstrip` setting

> **Removed 2026-09-24.** The setting and its `multi-only` test are gone: a
> Pane header always shows its tabs. See
> `SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md` §8.
> The text below records the original task.

Add the setting (default `always`, per §7 resolution 1) to wherever AgentMux's
settings schema/defaults live (find the existing pattern — likely alongside other
`pane:*`/`widget:*` keys already used in `settings.json`, see `widget:pinned`/
`widget:icononly` for the naming convention). `PaneHeaderTabStrip` reads it to
decide whether a single-tab Pane shows a plain title or a one-pill strip — same
row either way (§4.1), never a second row.
**Test:** settings default test (a fresh settings.json has `pane:tabstrip` absent
→ treated as `always`); a unit test for `PaneHeaderTabStrip` with the setting
forced to `multi-only` confirming single-tab renders a plain title, not a pill.

### A3. Generic "push onto an existing Pane's stack" — CONFIRMED zero backend work needed
Verified directly against agent's own existing "+" handler
(`agent-view.tsx:606-650`, `handleNewAgentTab`) before writing anything: the
existing `pane.open` RPC already supports exactly this generically via
`skip_placement: true` — it creates the block WITHOUT placing it anywhere, and the
frontend then calls `pushBlockOntoStack` itself. No new backend parameter, no new
RPC. The full, confirmed sequence (extract into a shared helper,
`addWidgetAsPaneTab(layoutModel, nodeId, blockDef)`, in
`frontend/layout/lib/layoutStack.ts`, so A4 and any future caller don't
re-duplicate agent's inline version a third time):

```ts
export async function addWidgetAsPaneTab(model: LayoutModel, nodeId: string, blockDef: BlockDef): Promise<void> {
    const paneOpenResult = (await TabRpcClient.rpcCall(
        "pane.open",
        { view: blockDef.meta?.view, skip_placement: true, meta: blockDef.meta },
        {}
    )) as { block_id: string };
    // Re-resolve the node fresh — it may have closed while the RPC was in
    // flight (same race agent's own handler already guards, agent-view.tsx:617-621).
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node) {
        await ObjectService.DeleteBlock(paneOpenResult.block_id).catch(() => {});
        return;
    }
    pushBlockOntoStack(model, nodeId, paneOpenResult.block_id);
}
```

**Test:** a `layoutStack.test.ts` (or wherever `layoutStack.ts`'s existing tests
live, if any — check first) case: call `addWidgetAsPaneTab` against an existing
Pane, assert `block_stack` grew by one and `active_block_id` points at the new
block. A second case for the race guard: simulate the node disappearing between
the RPC and the re-resolve, assert `DeleteBlock` is called and nothing throws.

### A4. Generic "+" picker — DEFERRED to the follow-up plan, decided mid-implementation
Per design spec §4.5, the eventual goal is `PaneHeaderTabStrip`'s "+" opening a
`buildPaneWidgetMenuItems`-based picker across every rolled-out widget type. For
THIS pass, agent/term's "+" keeps its existing, proven, single-purpose behavior
(fork a new agent / open a new shell, no menu) rather than being replaced with a
picker prompt — that would be a real UX regression for the single most common
case (adding another instance of the SAME type) in exchange for a capability
(picking a DIFFERENT type) that has limited value until Task Group C's deferred
scope (browser/editor/etc. onboarding) actually lands. Revisit A4 as part of that
follow-up work, not this pass. Note for that future work: `pane-leaf-chrome.tsx`'s
swap-one-at-a-time fallback render path is already view-agnostic (§2.4), so a
generic picker could technically push a NOT-YET-hoisted widget type onto an
agent/term pane's stack today and it would render correctly (just without stable
chrome across switches) — worth confirming this actually works end-to-end before
building A4 for real, since it changes how urgently browser/editor chrome-hoisting
needs to land first.
**Test (when this is picked back up):** click "+", pick a widget type, assert a new pill appears in the SAME
Pane (not a new split) and becomes active.

---

## Task Group B — Migrate agent + term onto the unified chrome (the reference implementation)

This is where the "two rows become one" restructuring (§4.1, §2.4) actually lands,
for the two widget types that already have SOME form of in-pane tabs today.

### B1. `pane-leaf-chrome.tsx` router update
Replace `HOISTS_OWN_CHROME`'s role: instead of gating "does this view type get
stable chrome at all," it (renamed `HOISTS_ROLLED_OUT` for clarity, or kept as-is —
implementer's call, not worth its own spec debate) now gates "does this view type
render via the new `PaneHeaderTabStrip` path." Starts as `{"agent", "term"}` —
unchanged membership for this task group, since B2/B3 only migrate those two; the
membership only grows in Task Group C.

### B2. Retire `AgentPaneChrome`'s two-row structure
In `frontend/app/view/agent/agent-view.tsx` (`AgentPaneChrome`, lines 350-895 as of
this plan's writing — re-verify current line numbers before editing, per the
ReAgent finding on the design spec's own citations drifting): replace the
`headerElem` (built from `BlockFrame_Header`, lines ~427-435) + separate
`.agent-pane-stack-content` wrapping div + `PaneTabStrip` render (lines ~840, ~848,
~867) with a single `PaneHeaderTabStrip` instance. Preserve every existing behavior
this component's extensive doc comments describe as load-bearing (re-read them
before touching anything — this file has multiple "reagent P0/P1/Codex P2" comments
documenting real, already-fixed bugs; e.g. the focus-ring/`isAlone` logic, the
`noHeader` suppression flag, the "stack" vs. "history" tab merge logic) — this is a
structural relocation of existing, tested behavior, not a rewrite of the behavior
itself.
**Test:** existing agent-pane tests (find and run them — likely
`agent-view.test.tsx` or similar) must still pass unmodified (proves behavior
parity). Add one new test: a single-conversation agent pane renders exactly one
`PaneHeaderTabStrip` row, not a header-plus-strip pair.

### B3. Retire `TermPaneChrome`'s two-row structure
Same operation as B2, applied to `frontend/app/view/term/term.tsx`'s
`TermPaneChrome` (lines 442-751 as of this plan's writing). Terminal's
`KEEP_ALIVE_TYPES` membership (§4.3) is UNCHANGED by this task — B3 only touches
the chrome/header structure, not the mount-lifecycle behavior.
**Test:** existing terminal-tab tests must still pass unmodified; add the same
single-row-render assertion B2 added, for a terminal pane.

### B4. Visual/manual verification pass (the part I cannot fully self-verify)
Run `task dev` and manually exercise: an agent pane with 1 conversation (should
look like today's plain header, just built from the new component); an agent pane
with 3+ forks (tab strip visible, switching works, close-tab vs close-pane both
work correctly); same two checks for terminal. **I do not have screen access — I
can confirm via logs that no runtime errors/console warnings fire during these
interactions and that the DOM structure (checked via a UIQuery-equivalent or a
targeted test) matches "one row," but I cannot confirm the VISUAL result looks
correct at a glance the way a human can.** This is the task where the repo owner's
own local look-and-feel pass matters most before any merge decision (matches the
"test rigorously locally" instruction directly) — flagged here explicitly rather
than silently claimed as fully verified.

---

## Task Group C — extended to every remaining widget type (done, 2026-09-17)

**Originally scoped as "explicitly deferred" — revised live** after the repo owner
tested Task Groups A/B locally and asked directly: *"don't all panes need to be
generalized to this?"* Confirmed yes (that's the design spec's whole point), and
was asked to keep going rather than stop at agent/term.

**What actually shipped, and how** (materially different from — and cheaper than —
the "9 bespoke Chrome components" the original Task Group C draft implied):
instead of writing a separate `XPaneChrome` per remaining widget type (mirroring
`AgentPaneChrome`/`TermPaneChrome`'s domain-specific richness), built ONE shared
`genericRenderPaneChrome` (`frontend/app/element/GenericPaneChrome.tsx`) using only
the already view-agnostic primitives `layoutStack.ts`/`action-widgets-config.ts`
already provided — `blockStack` for tabs, `setActiveBlockInStack`/
`closeBlockInStack` for switch/close, `buildPaneWidgetMenuItems` +
`addWidgetAsPaneTab` (Task A3, reused as originally designed) for "+" (opens the
SAME native widget picker the widget bar's own right-click menu already builds, at
the "+" click position — required extending `PaneTabStrip`'s `onAdd` signature to
optionally receive the `MouseEvent`, a backward-compatible addition every existing
caller ignores). Every one of browser/editor/sysinfo(+its `cpuplot` alias)/swarm/
armory/media/drone/help/warden registers this SAME function via a one-line
`renderPaneChrome = genericRenderPaneChrome` field in its own `ViewModel` class —
see any of those 9 files for the pattern (3 of them — media/help/sysinfo — also
needed their constructor extended to actually accept the `nodeModel` param
`block.tsx`'s `makeViewModel` always passes positionally as arg 2 to EVERY
registered ViewModel class regardless of its own declared signature; sysinfo's own
constructor had been silently receiving that NodeModel object into a param
misnamed/mistyped `viewType: string` the whole time — a real, harmless-in-practice
pre-existing bug found and fixed as a necessary byproduct, not a gratuitous
refactor).

Also required (and easy to miss — caught this exact gap on the first `task dev`
pass, not by reading alone): every newly-hoisted `ViewModel` needs its own
`noHeader = () => this.nodeModel.paneChromeHoisted === true` field too, mirroring
`AgentViewModel`'s/`TermViewModel`'s identical existing field — `pane-leaf-chrome.tsx`'s
own doc comment for `HOISTS_OWN_CHROME` says so explicitly ("MUST also set
`noHeader`... or its inline BlockFrame header and its hoisted one will both
render"), and it is genuinely required: without it every one of these 9 panes
would show a duplicate header. Verified this is NOT happening via `task dev` (a
`grep` for "paneChromeHoisted" across the running dev log showed one transient
HMR-staleness false-positive from an old, pre-edit `SysinfoViewModel` instance —
resolved by a full restart, confirmed clean on fresh launch).

`HOISTS_OWN_CHROME` (`pane-leaf-chrome.tsx`) now lists all 12 keys: `agent`,
`term`, `browser`, `editor`, `sysinfo`, `cpuplot`, `swarm`, `armory`, `media`,
`drone`, `help`, `warden`.

**Still genuinely deferred** (unchanged from the original plan, none of this
shipped in this pass):
- The new keyboard shortcuts from §4.9's resolved table (`Ctrl:Shift:]`/`[`,
  `Ctrl:Shift:T`, `Ctrl:Alt:[`/`]`, and `Cmd:w`'s redefinition).
- Tab reorder (drag + keyboard) and tear-off-from-stack (§7 resolution 5) — both
  explicitly gated behind the separate drag-session refactor per design spec §4.8.
- Editor's own files-tabs migration onto real `blockStack` semantics (§7
  resolution 3) — editor now HAS the outer unified header (correctly showing/
  hiding based on whether OTHER widget types have been pushed onto its Pane's
  stack via the generic "+"), but its own internal multi-file tab strip
  (`editor-tab-strip.tsx`, `editor-pane-state-store.ts`) is untouched and remains
  a structurally separate mechanism, not yet unified into `blockStack` members.
- **Deep, per-widget-type manual QA.** Verified via `task dev` that nothing
  crashes and the app launches/HMRs cleanly with all 12 view types hoisted, but did
  NOT individually click through browser/sysinfo/swarm/armory/media/drone/help/
  warden's own "+"-opened widget picker, tab-switching, and tab-closing behavior
  one by one — that is real, still-needed verification work, explicitly flagged
  rather than claimed as done. A `GenericPaneChrome`-specific automated test (unit
  tests exist for `PaneHeaderTabStrip` and the hoisting router, but not yet for
  `genericRenderPaneChrome`'s own tab-derivation/add/close logic) is also a
  legitimate gap — noted here rather than silently skipped.

---

## Verification checklist (run after every task, not just at the end)

- [ ] `npx tsc -p tsconfig.citypecheck.json --noEmit` clean (excluding the known
  pre-existing `.bench.ts` errors from unrelated files)
- [ ] `npx vitest run frontend/app/element frontend/app/view/agent frontend/app/view/term frontend/app/tab` — all existing + new tests pass
- [ ] `cargo check --workspace --tests` clean (only if A3 turns out to need backend work)
- [ ] `task dev` manual pass per B4, with console/log inspection for runtime errors
- [ ] No change to any file this plan doesn't name, confirmed via `git status`/`git diff --stat` before each commit
