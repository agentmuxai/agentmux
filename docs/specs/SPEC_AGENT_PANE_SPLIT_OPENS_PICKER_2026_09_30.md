# Spec: Splitting an agent pane opens a fresh agent picker

**Status:** implemented (this PR)
**Date:** 2026-09-30
**Author:** Lark

## Ask

In an agent pane, the right-click menu's **Split Up / Down / Left / Right**
should open the agent picker: the "My Agents" list with "New Agent" below it.
That is the same view "+" → Agent opens.

## Today

`handleSplitPane` (`frontend/app/block/pane-actions.ts`) copies the source
pane's whole meta into the new pane. A view can list keys the copy must skip
(`splitDropsMeta`). The agent view skips its 13 agent-identity keys
(`AGENT_SPLIT_DROPPED_META`), so `agentId` is unset and the picker shows. The
rest still comes across, because a deny-list only drops what it names:

- **`agent:historyTabFor`.** A split of an agent's history tab is another
  history reader, not the picker (`agent-block-content.tsx` checks this key
  before `agentId`).
- **`agent:runtime`.** The source agent's per-pane runtime overrides, such as
  model and effort. An agent launched from the new pane's picker starts with
  them (`buildRuntimeArgs.ts`).
- **`agent:sessionid`, `agent:last_failure`, `agent:provider_flags`,
  `agent:virt:partition`, `agent:livefeed*`, `agent:stashheight`**, and any key
  added later. They're per-session state of a pane the new one has nothing to
  do with.

Each new per-pane `agent:*` key would have to be added to the list by hand.

## Change

A view can say what a split of it creates, instead of what it doesn't copy:

- **`PaneTabCapabilities.splitBlockDef?: () => BlockDef`** replaces
  `splitDropsMeta`. The agent view was its only user. When a view declares it,
  `handleSplitPane` creates that block, in the chosen direction. Nothing
  comes from the source pane. Views without it are unchanged: they copy the
  meta, drop a local `connection` and the floating "Always on top" tack.
- **The agent view's `splitBlockDef` is `agentPickerBlockDef()`,** the
  configured Agent widget's blockdef (`defwidget@agent`). "+" → Agent uses
  it, and `close-agent-tab.ts` already opens it in place of a closed agent
  tab. It moves from `close-agent-tab.ts` into its own module, which both
  callers share. It returns a copy, so a caller can't change the config.
- `AGENT_SPLIT_DROPPED_META` is deleted.

All four directions behave the same. The source pane's session is untouched.

## Out of scope

- The command palette's and keybindings' `split:*` commands. They split the
  focused pane into a terminal whatever its view (`getDefaultSplitBlockDef`,
  `command-registry.ts`). That's a separate choice of default.
- Which agent the picker suggests. The new pane is a plain, fresh picker.

## Tests

`pane-actions.test.ts`:
- Split Down on an agent pane, including a history tab carrying
  `agent:historyTabFor`, `agent:runtime` and `agent:sessionid`, creates
  exactly the Agent widget's blockdef, below the source block.
- All four directions do the same.
- The blockdef handed over is a copy.
- A terminal split still copies its meta and drops the tack (existing test).

`block-registry.test.ts`: `splitBlockDef` is declared by exactly the agent
view, replacing the `splitDropsMeta` assertion.

## Files

- `frontend/app/block/pane-tab-registry.ts`: `splitBlockDef` replaces
  `splitDropsMeta`.
- `frontend/app/block/pane-actions.ts`: `handleSplitPane` uses it.
- `frontend/app/view/agent/agent-picker-blockdef.ts` (new):
  `agentPickerBlockDef`.
- `frontend/app/view/agent/agent-manifest.tsx`, `close-agent-tab.ts`,
  `agent-pane-tab.ts`: use it; drop `AGENT_SPLIT_DROPPED_META`.
- `docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`: the capability name.
