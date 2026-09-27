# SPEC: one way to reveal a block — including a background tab in a multi-tab pane

**Author:** lark
**Date:** 2026-09-27
**Status:** active — Phase 1 (revealBlock, Swarm + token popover) shipped in PR #3961; Phase 2 (move the other callers, delete duplicates), Phase 3 (other windows) and the optional MCP tool not started.
**Related:** `SPEC_SWARM_ROW_AGENT_COLOR_AND_SELECT_TO_FOCUS_2026_09_25.md` §2.3 (Swarm select-to-focus),
`SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md` (token popover → agent pane), the OS-notification
rich-content spec (`notification:activate`, `os-notify-bridge.ts`), the in-pane tabs work
(`frontend/layout/lib/layoutStack.ts`, `stackMembers.ts`).

All line numbers are against `main` at `135251326`.

---

## 1. Request (user, 2026-09-27)

> "Selecting an agent entry in the Swarm whose agent is in an unfocused tab of a pane selects the pane, but not
> the tab. Refine that. It may need some architecture rethink, or DRY opportunities."

---

## 2. Root cause

Swarm rows call `focusBlock(blockId)` (`frontend/app/util/focus-block.ts:23`). Its fast path:

```ts
const node = layoutModel?.getNodeByBlockId(blockId);
if (node?.id != null) {
    await setActiveTab(tabId);
    layoutModel.focusNode(node.id);
    return;
}
```

- `getNodeByBlockId` deliberately matches a **background** member of a multi-tab pane
  (`layoutNodeModels.ts:222-231`: `leaf.data.blockId === blockId || leaf.data.blockStack?.includes(blockId)`), so
  it returns the **pane**, not the member.
- `focusNode` (`layoutFocus.ts:153`) only sets which pane is focused. It never changes the pane's active member
  (`TabLayoutData.activeBlockId`), and it returns early if the pane is already focused.
- Nothing else in `focusBlock` touches the stack. So the pane is selected and its visible tab stays whatever it
  was. That is exactly the reported symptom.

The function that does switch a pane's tab exists: `setActiveBlockInStack(model, nodeId, blockId)`
(`layoutStack.ts:142-160`). It is what a click on a pane-header tab pill ends up calling
(`PaneChrome.tsx:288-295`), and it's a no-op when the block is already active.

---

## 3. The same job is done five different ways today

"Take me to this block" is implemented separately in several places, each covering a different subset of the
cases (another window tab, another window, a background tab inside a pane). None covers the third.

| Path | Callers | Other window tab | Other window | Background pane tab | Caret |
|---|---|---|---|---|---|
| `focusBlock` (`util/focus-block.ts:23-64`) | Swarm rows (`swarm-view.tsx:318`), token popover (`TokenBreakdownPopover.tsx:160`) | only if that tab's layout is already loaded; the lookup misses unloaded tabs and the slow path skips the current workspace | activates the tab, raises the window, **no pane focus** | **no** | no `giveBlockFocus` |
| `activateBlockLocally` / `focusBlockInTab` (`notification/os/os-notify-bridge.ts:114-154`) | OS notification clicks | **yes**: finds the tab via `Tab.blockids`, retries until the layout exists | **yes**: srv resolves the window and tells *that* renderer (`notification:activate`) | **no** | yes |
| `refocusNode(blockId)` (`store/block-component-registry.ts:170-183`) | AgentPicker "switch to existing" (`:538`), agent fork pill (`agent-view.tsx:465`), browser (`browser-model.ts:69`), `globalRefocus` (`keymodel-nav.ts:145`) | no (active tab only) | no | **no** | yes |
| `setActiveBlockInStack` direct (`PaneChrome.tsx:294`, `agent-view.tsx:463`, `open-history-tab.ts:101`) | tab pill clicks, fork/history tabs | no | no | yes (own pane) | varies |
| MCP `SetActiveTab` / `FocusWindow` (`agentmux-mcp/src/main.rs:2198`, `:2247`) | agents | tab only | window only | no | no |

`tab-reveal.ts` sounds relevant but isn't: it's the anti-flicker visibility gate around tab and pane switches
(`holdRevealGate`, `holdLeafRevealGate`), not a way to reveal a block.

The best of these, `activateBlockLocally`, already solves the hard parts (unloaded tabs, other windows, caret)
and just lacks the stack step. That's the base to build on.

---

## 4. Design

### 4.1 One function: `revealBlock`

New module `frontend/app/util/reveal-block.ts`, replacing `focus-block.ts`:

```ts
export interface RevealOptions {
    /** Tab that holds the block, when the caller already knows it (srv-resolved). */
    tabId?: string;
    /** Put the caret in the block (its composer / terminal). Default true. */
    focusCaret?: boolean;
}

/** Reveal `blockId` in THIS window: activate its window tab, switch its pane to
 *  it if it's a background pane tab, focus the pane, and (by default) the caret.
 *  Returns false when the block isn't in this window's workspace. */
export async function revealBlockLocally(blockId: string, opts?: RevealOptions): Promise<boolean>;

/** Reveal `blockId` wherever it is: here if possible, else in the window that
 *  shows it (§4.3). */
export async function revealBlock(blockId: string, opts?: RevealOptions): Promise<void>;
```

`revealBlockLocally`, in order:

1. **Find the tab** through `Tab.blockids` (pinned tabs first, then regular), as `activateBlockLocally` does, not
   through `getLayoutModelForTabById`. That also finds tabs whose layout hasn't been built yet. `opts.tabId`
   short-circuits the search when valid.
2. **`setActiveTab(tabId)`** if it isn't already active.
3. **Wait for the layout model** to contain the block: poll `getNodeByBlockId` up to 10 × 50 ms (the existing
   `focusBlockInTab` loop). Give up quietly and return `false` if it never appears.
4. **Un-magnify** if a *different* pane in that tab is magnified; otherwise the target stays hidden behind it
   (`focusNode` doesn't un-magnify).
5. **Switch the pane's tab:** `setActiveBlockInStack(layoutModel, node.id, blockId)`. This is the missing step.
   It's a no-op for a single-block pane or an already-active member, so it's safe to always call. Call the stack
   function directly, not the agent pane's `onActivate` hook: that hook exists to handle cross-pane forks started
   from a pill click, which isn't this case.
6. **`focusNode(node.id)`** to select the pane.
7. **`giveBlockFocus(blockId)`** when `focusCaret !== false`. It must come **after** step 5: a kept-alive hidden
   member is still registered and would otherwise take the caret while invisible. It also covers `focusNode`'s
   early return when the pane was already focused.

Optional polish: wrap steps 5–6 in `holdLeafRevealGate` / `scheduleLeafRevealLift` (as `open-history-tab.ts:96`
does) if the switch visibly flickers in testing.

### 4.2 Everyone uses it (the DRY part)

| Today | Becomes |
|---|---|
| `focusBlock(blockId)` in Swarm and the token popover | `revealBlock(blockId)` |
| `activateBlockLocally(blockId, tabId)` / `focusBlockInTab` in `os-notify-bridge.ts` | `revealBlockLocally(blockId, { tabId })`; the local helpers are deleted |
| `refocusNode(blockId)` in AgentPicker's "Switch to existing" and the agent fork pill (a fork in another pane) | `revealBlock(blockId)` / `revealBlockLocally(blockId)`: the target may be in another window tab or a background pane tab. The browser pane's own click and `globalRefocus` keep `refocusNode`: they re-focus a pane that is already on screen, which `refocusNode` does correctly and synchronously |
| `util/focus-block.ts` | Phase 1 kept `focusBlock` as an alias; Phase 2 moves its two callers to `revealBlock` and deletes the file |

`openOrFocusPaneByView` (focus an existing pane of a view type or create one) and direct
`setActiveBlockInStack` calls for a pane's *own* tabs are different jobs and stay.

### 4.3 Another window

Today's slow path activates the tab in the other window and raises it, but can't select the pane or its tab: a
renderer can't drive another window's layout model. The OS-notification path already has the working pattern:
**srv resolves which window shows the block and tells only that window's renderer to reveal it itself.** Reuse
it rather than adding a second mechanism:

- New RPC `block.reveal { block_id }` in srv. It resolves the block's tab / workspace / open window with the
  resolver notifications already use (`notify/router.rs` `resolve_click_target`), then:
  - **Window open:** publish an event `block:reveal { block_id, tab_id, window_id }`. The named window's renderer
    handles it with `revealBlockLocally(block_id, { tabId })`. srv also raises that window (the same path
    notification clicks take through the launcher).
  - **Workspace open nowhere:** open it the same way a notification click does (park and open), then reveal on
    arrival.
- **The stack member is also switched server-side**, before the event goes out, through the existing
  `LayoutStackActivate` reducer command (`reducer/layout.rs:571-581`, `backend/layout/mod.rs:235-248`). The
  stored tree then already has the right member active, so a window that loads the tab fresh shows the right tab
  even before it handles the event. (`pane.moveTab` with `block_id == target_block_id, activate: true` reaches
  the same function today, but it's a reorder API; a named call is clearer.)
- `notification:activate`'s handler can then route through the same `revealBlockLocally`, so there is one
  receiving path too.

`revealBlock` = `revealBlockLocally` first; if that returns `false`, call `block.reveal`.

### 4.4 Agents (MCP), optional

A thin MCP tool `RevealBlock { block_id }` over `block.reveal` would let an agent bring any pane to the front
precisely. The existing `SetActiveTab` / `FocusWindow` tools stay. This is optional; it's listed so the RPC is
designed for it.

---

## 5. Phases

1. **Phase 1, the fix:** `reveal-block.ts` with `revealBlockLocally` (§4.1) and `revealBlock` using today's slow
   path unchanged. Swarm and the token popover switch to it. This alone fixes the reported bug for the current
   window.
2. **Phase 2, DRY:** move the notification bridge and `refocusNode(blockId)` callers onto `revealBlockLocally`;
   delete `focus-block.ts` and the bridge's local helpers.
3. **Phase 3, other windows:** `block.reveal` RPC + `block:reveal` event + server-side stack activation (§4.3).
4. **Optional:** the MCP tool (§4.4).

Each phase is its own PR.

---

## 6. Tests

There is no `focus-block.test.ts` today, and every current caller mocks `focusBlock` out. Phase 1 adds
`reveal-block.test.ts`, which records calls on a fake layout model with `setActiveBlockInStack` stubbed. What it
pins is what `revealBlockLocally` adds, the steps and their order. The stack switch itself is tested against a
real `LayoutModel` in `layout/tests/layoutStack.test.ts` (`setActiveBlockInStack`); PR #3961 adds the case
`revealBlockLocally` relies on, since it calls the switch on every reveal: it persists only on a real switch,
never for the already-active tab or a single-block pane (a `persistToBackend` spy).

`reveal-block.test.ts` covers:

- a background member of a multi-tab pane: window tab activated, then `setActiveBlockInStack(pane, block)`, then
  `focusNode(pane)`, then `giveBlockFocus(block)` — the caret strictly **after** the stack switch;
- a block in a not-yet-loaded window tab: found via `Tab.blockids`, tab activated, polled until the layout exists;
- a different pane magnified: un-magnified first; the target's own magnify left alone;
- a block not in this window: returns `false` with no side effects;
- `focusCaret: false`: no `giveBlockFocus`;
- a `tabId` hint is tried first.

Swarm's side (`focusedActiveBlockId`) has its own test: the selection follows a tab switch inside the same
focused pane.

Phase 2: the notification bridge's tests assert it delegates to `revealBlockLocally`. Phase 3: srv tests for
`block.reveal` (resolves window and tab, activates the stack member in the stored tree, publishes
`block:reveal` naming only that window), plus a renderer test for the event handler.

Live check (Phase 1) in a dev build: put two agents as tabs in one pane, leave agent A's tab showing, click agent
B in the Swarm. Pane B's tab is shown and focused, the caret is in B's composer, and no Swarm border flashes (the
separate #3955 fix).

---

## 7. Non-goals

- Changing how a click on a pane-header tab pill works.
- `openOrFocusPaneByView`.
- Keyboard navigation between panes.
