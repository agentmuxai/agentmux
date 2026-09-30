# Spec: Splitting an agent pane opens a fresh agent picker

**Status:** implemented (#4077, every split path: #4093)
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

## Every split path (amendment, 2026-09-30)

#4077 covered only the pane menu. The repo owner asked for this to hold
however the split is asked for. There are three split paths. Each asks the
same helper, `splitBlockDefFor(source, fallback)`
(`frontend/app/block/split-block-def.ts`). A view that declares
`splitBlockDef` gets it, and any other view gets the caller's own default:

| Path | Where | Default for other panes (unchanged) |
|------|-------|-------------------------------------|
| Pane menu: Split Up / Down / Left / Right | `pane-actions.ts` | a copy of the pane's meta |
| Command palette: `split:up/down/left/right` | `command-registry.ts` | a terminal |
| Shortcuts: Cmd+D, Shift+Cmd+D, Ctrl+Shift+S then an arrow | `keymodel-blockcreate.ts` | the default new block (`app:defaultnewblock`: a terminal in the focused pane's cwd, or the launcher) |

The palette and the shortcuts split the focused pane, so a focused agent pane
splits into a fresh picker. Not covered:

- **New Block** (Cmd+N). It isn't a split, and still opens the default new
  block.
- **`/terminal`**. The agent slash command asks for a terminal by name.

## Out of scope

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

`split-block-def.test.ts`: a declaring view gets its block without building
the fallback. Other views, and no source block, get the fallback.

`keymodel-blockcreate.test.ts`: the split shortcuts split a focused agent pane
into its picker in all four directions. A terminal still splits into a
terminal in its cwd. With nothing focused there's no split. Cmd+N on an agent
pane still opens the default new block.

## Files

- `frontend/app/block/pane-tab-registry.ts`: `splitBlockDef` replaces
  `splitDropsMeta`.
- `frontend/app/block/pane-actions.ts`: `handleSplitPane` uses it.
- `frontend/app/view/agent/agent-picker-blockdef.ts` (new):
  `agentPickerBlockDef`.
- `frontend/app/view/agent/agent-manifest.tsx`, `close-agent-tab.ts`,
  `agent-pane-tab.ts`: use it; drop `AGENT_SPLIT_DROPPED_META`.
- `docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`: the capability name.
