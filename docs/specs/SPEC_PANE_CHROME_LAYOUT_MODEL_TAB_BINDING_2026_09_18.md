# SPEC: A pane's chrome must resolve its own tab's LayoutModel, never "whichever tab is globally active"

**Date:** 2026-09-18
**Status:** implemented — PR #3392.
**Related:** `docs/specs/SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`
(the chrome-stability design whose "constructed once, never remounts"
contract makes a wrong one-time capture permanent),
`docs/specs/SPEC_TAB_CREATION_REVEAL_ARCHITECTURE_2026_09_16.md` (the
deliberate `activate: false` new-tab sequencing that triggers this every
time). An unrelated, concurrently-developed fix in the same file family
(agent-tab keep-alive in the pane tab strip) was in flight the same day on
a separate branch/PR — no interaction between the two; not cited by path
here since it hadn't merged as of this spec.

---

## 0. The ask

> opened a new window tab, loaded Posa in an agent pane, and try to add
> another pane tab to the pane posa is in, trying to add anything is a
> no-op .. is there an error in the logs?

Confirmed live, on a shared multi-agent instance and reproduced again in an
isolated `task dev` instance: clicking "+" on a newly-created tab's default
agent pane silently did nothing. Same failure for every widget type tried
(agent, drone, warden), not agent-specific.

---

## 1. What the logs showed

Every "+" click produced this exact pair, live-correlated by watching both
the srv and host logs while reproducing on request:

```
INFO  pane.open view=agent
INFO  pane.open: block created, placement skipped   block_id=<X>
...
ERROR [fe] Cannot apply eventbus layout action DeleteNode, could not find
      leaf node with blockId <X>
```

`skip_placement` (`agentmux-srv/src/server/app_api/mod.rs`) is not itself a
bug — it's the normal first half of "add this widget to an existing pane's
stack": create the block without placing it in any tree, and the caller
(`addWidgetAsPaneTab`, `frontend/layout/lib/layoutStack.ts`) pushes it onto
the target leaf's `blockStack` itself. The second half is what failed:
`addWidgetAsPaneTab`'s own `findNode(model.treeState.rootNode, nodeId)`
returned `null` — the leaf genuinely wasn't in `model`'s tree — so it fell
through to its "orphaned, clean it up" branch (`ObjectService.DeleteBlock`),
which triggers the `delete_block` saga
(`agentmux-srv/src/sagas/delete_block.rs`), which queues a
`pendingbackendactions` `DeleteNode` entry for whichever tab the block
actually lives in. When THAT tab's own `LayoutModel` later drained its
queue, it tried to prune a leaf that was never placed in ITS tree either
(skip_placement never placed it anywhere) — hence the `[fe]` error, with no
notification ever reaching the user (the `addWidgetAsPaneTab` not-found
branch returns normally; nothing throws).

The question this answers: `model` — the `LayoutModel` `addWidgetAsPaneTab`
was called with — was bound to the **wrong tab** for this specific pane, and
had been since the moment its chrome first constructed.

---

## 2. Root cause

Three functions independently resolved "the LayoutModel this pane's chrome
should act against" via:

```ts
const layoutModel = getLayoutModelForStaticTab();
```

— `renderPaneChromeShell` (`frontend/app/element/PaneChrome.tsx`),
`buildAgentPaneChromeModel` (`frontend/app/view/agent/agent-view.tsx`),
`buildTermPaneChromeModel` (`frontend/app/view/term/term.tsx`) — plus a
fourth, `PaneLeafChrome`'s own `stackBlockIds` lookup
(`frontend/app/tab/pane-leaf-chrome.tsx`).

`getLayoutModelForStaticTab()` (`frontend/layout/lib/layoutModelHooks.ts`) is
`atoms.activeTabId()` read once, non-reactively — "whichever tab the app is
showing right now." Per
`SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`, a pane's chrome
constructs exactly once and never remounts. Combining the two: whichever tab
happened to be active at the single moment chrome first constructed is
**permanently** what every subsequent "+"/close/switch on that pane targets
— correct only as long as that never diverges from the tab the pane actually
lives in.

It diverges every time a tab is created. `createTab()`
(`frontend/app/store/tab-actions.ts`) creates the new tab with
`activate: false`, then calls `applyTabPreset(tabId, DEFAULT_TAB_PRESET)` —
which builds the tab's starter layout, **including its default agent
pane** — and only calls `setActiveTab(tabId)` once that finishes. The
window-level tab bar keeps every tab's content mounted simultaneously (only
toggling visibility, the same `display:none`-and-never-unmount pattern
`SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` documents for that surface),
so the default agent pane's `Block`/`ViewModel`/chrome construct **while the
tab is still inactive**, every single time a tab is created — not a rare
race, the deterministic normal case. `getLayoutModelForStaticTab()` at that
moment returns the *previous* tab's model, and chrome latches it forever.

Manually created panes (added to an already-active tab) never hit this: by
construction, a pane the user just added lives in whatever tab was already
active when they added it.

---

## 3. The fix

Give `NodeModel` (`frontend/layout/lib/types.ts`) its own `layoutModel`
field, populated at construction (`getNodeModel`,
`frontend/layout/lib/layoutNodeModels.ts`) from the very `LayoutModel`
parameter that function already receives — which IS the correct, tab-scoped
model for the node being built, by construction, with no lookup and no
timing dependency (the same "resolve via the leaf's own stable identity, not
a global proxy for it" fix `anchorBlockId`'s own doc comment in
`agent-model.ts` already applied to blockId; this is the same lesson applied
to the pane's tab).

All four call sites now read `nodeModel.layoutModel` instead of calling
`getLayoutModelForStaticTab()`. The keep-alive wrapper `NodeModel`s
(`pane-leaf-chrome.tsx`'s `scopedNodeModel`/`keepAliveNodeModelFor`) needed
no change — they spread the real leaf `nodeModel`, which already carries the
field through.

**Verification:** reproduced live (multi-agent shared instance, then again
in an isolated `task dev` instance built from this fix), confirmed the exact
log signature above, applied the fix, hot-reloaded, and confirmed the same
"+" click on a freshly-created tab's default agent pane now correctly adds a
new pane tab with no `DeleteNode` error in either log.

Regression coverage: `frontend/app/element/PaneChrome.test.tsx`'s "resolves
the OWNING tab's LayoutModel, never the globally-active one" suite —
`getLayoutModelForStaticTab` is mocked as a spy returning a distinguishably
wrong model, and every mutating handler (`onActivate`/`onClose`/`onAdd`) is
asserted to operate on `nodeModel.layoutModel` and to never call the spy at
all.
