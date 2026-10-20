# RCA: tearing a pane with several pane tabs out of the window moves only its visible tab

**Status:** analysis. Fixed by the PR that carries this file (section 8 says how). Written 2026-10-19; the code reading was checked against `main` at `f77c81f50`. Nothing here was reproduced in a controlled run before the fix; section 2 says what the observation rests on.
**Author:** Maop@starpower, at the operator's request
**Trigger (operator, 2026-10-19, testing a dev build on macOS):** "If I tear out a pane tab, it works nice, but if I tear a pane with multiple pane tabs, it appears to tear off only the selected pane tab. It should actually tear the entire pane (including all pane tabs in that pane). Investigate whether this is cross platform too."
**Related:** [`SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md`](../specs/SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md) §3.5 (the tear-off design, which names this as a known gap), [`SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`](../specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md), [`SPEC_PANE_TEAROFF_MOTHER_RESIZE_2026_06_20.md`](../specs/SPEC_PANE_TEAROFF_MOTHER_RESIZE_2026_06_20.md)

## 1. Summary

**It is cross-platform, and it is not a regression.** Every tear-off carries exactly one block, on macOS, Windows and Linux alike. A pane that holds several pane tabs is a single layout leaf with a `blockStack`; dragging it out by its header builds a payload that names only the leaf's visible block (`node.data.blockId`), and the rest of the chain (the srv saga, the new window's layout, the floating window itself, redock) only knows how to move one block. So the floater gets the visible tab, and the other tabs stay docked in the source pane.

Dragging a single pill out and dragging the whole pane out by its header therefore do the same thing for a multi-tab pane. That is correct for the pill and wrong for the header.

It was never right. Before #3708 (2026-09-24) the source window deleted the whole leaf after the tear-off, so the other tabs vanished from the layout; #3708 fixed that by leaving them docked, and wrote the whole-stack case down as "noted, not addressed" (spec §3.5 design 3, and the spec's non-goals list). The operator is now seeing the documented gap.

A fix is not a frontend-only change: the saga, the new tab's layout, the floating window and redock all assume one block. Section 5 lists the places.

## 2. Observed

- A single pill dragged out of the window becomes a floating pane with that one tab. Correct.
- A pane with several tabs, dragged out by its header, becomes a floating pane holding only the visible tab. The other tabs are still in the source pane. (Reported by the operator; it matches the code path below.)
- The operator's dev instance logged two whole-pane tear-offs (`floating pane spawned (darwin)`) at 06:14 and 06:15 on 2026-10-19. The host log shows `TearOffBlock via saga` with one `block_id` each. That fits the reading but does not prove it: I did not capture the layout before and after, and I did not drive the operator's window.

## 3. Root cause

### 3.1 The chain, step by step

A pane's tab strip (`PaneTabStrip`) shows one pill per member of the leaf's `blockStack`. Two different drags start from it, and they set different payloads:

1. **A pill drag** sets `{ kind: "pane-tab", blockId, … }` (`PaneTabStrip.tsx:726`). One block is what the user grabbed, and one block is what moves. Correct.
2. **A header drag** (the grip or the empty header) is the whole-pane drag. `startTileDrag(node, model)` sets `{ kind: "tile", node, sourceTabId }` (`layout/lib/tile-drag.ts:26`). The payload carries the whole layout node, including `node.data.blockStack` and `activeBlockId`. The information is there.
3. On `dragend`, each platform's `CrossWindowDragMonitor` turns the node into one block id and drops the rest:
   - macOS `CrossWindowDragMonitor.darwin.tsx:163`
   - Windows `CrossWindowDragMonitor.win32.tsx:254`
   - Linux `CrossWindowDragMonitor.linux.tsx:158`

   ```ts
   const blockId = payload.node?.data?.blockId;   // the visible tab only
   dragPayloadForApi = { blockId };
   ```
   `node.data.blockId` is the visible member. The other members are in `node.data.blockStack` and are never read.
4. `performTearOff` calls `WorkspaceService.TearOffBlock(blockId, sourceTabId, sourceWsId, true)` (darwin `:233`, linux `:251`, win32 `:358`). The service takes one `block_id` (`crates/srv/src/server/service/tear_off.rs:29-44`).
5. The saga moves that one block: `CreateWorkspace`, `CreateTab`, `MoveBlock { block_id, … }` (`crates/srv/src/sagas/tear_off_block.rs:183`).
6. `setup_torn_off_block_layout` seeds the new tab with one leaf holding that block and no `blockStack` (`crates/srv/src/server/service/layout_helpers.rs:31-64`).
7. The source tab gives up that block only. `queue_source_layout_delete` and the window's `removeMovedBlock` are stack-safe by design: one member of a multi-tab pane leaves, and the pane and its other tabs stay (`layout_helpers.rs:232`, `layoutMagnify.ts:70-100`).
8. The floating window opens on `pane_id: blockId`.

At no step does anything name the other members. Each step is right for one block.

### 3.2 Why step 7 looks like a feature

#3708 made the source removal stack-safe so that tearing one pill out would not delete the pane's other tabs. The helper is shared by the pill path and the whole-pane path (the old `DeleteNode(getNodeByBlockId(...))` was replaced in all three monitors, `darwin:303`, `linux:334`, `win32:479`). That is the right behaviour for the pill. For a header drag it is why the rest of the pane stays docked: the source is doing exactly what it was told.

### 3.3 What the floating window assumes

The floating window is built around one block. `FloatingPaneWorkspace` reads `tab.blockids[0]` to find its pane (`floating-pane-workspace.tsx:94, 225, 441, 1000, 1057`), closes itself when the tab's `blockids` empties (`:119-123`), and redocks one block with `RedockFloatingPane(blockId, …)` (`:1000-1021`, and `CrossWindowDropOverlay.tsx:102`). Sending several blocks into that workspace without changing these would leave the extra tabs in `blockids` but unreachable for maximize, title and redock.

## 4. Cross-platform

| | macOS | Windows | Linux |
|---|---|---|---|
| Header drag builds a one-block payload | yes (`:163`) | yes (`:254`) | yes (`:158`) |
| `TearOffBlock` with that one block | yes | yes | yes |
| Other tabs left docked | yes | yes | yes |

Same behaviour on all three. The platforms differ only in the drag plumbing (host hit-test on Windows, DOM `dragend` coordinates on macOS and Linux).

**A second bug in the same path, Windows and Linux only (read from the code, not reproduced).** `measureMotherResize(payload.blockId)` runs for every whole-pane tear-off (`win32:349-353`, `linux:245-249`). It shrinks the source window by the pane's width when the pane spans the full height of the layout (`tear-off-pool-helper.ts:103-126`). It does not ask whether the pane is leaving. When a multi-tab pane that spans the full height has its visible tab torn off, the pane stays docked but the window still shrinks by its width. The pill path guards this with `wasOnlyTab` (`pane-tab-tearoff.ts:150-160`); the header path has no equivalent. macOS does not resize the source window at all, so it does not have this problem. A whole-stack fix removes it as a side effect, because the whole pane would then leave.

## 5. What a fix has to change

Tearing off the whole pane means moving every member of the stack, in order, with the same visible tab, and treating the pane as leaving the window. Moving the blocks themselves (not closing and reopening) is already the rule (spec §3.5, `MoveBlock` only reparents), so live agent sessions, terminals and editor buffers survive.

1. **Payload** (three monitors, one shared helper): read `effectiveStack(payload.node.data)` and `activeBlockId`; pass the members. Keep the single-block call as is when the stack has one member.
2. **Service and saga:** `TearOffBlock` needs a member list (or a sibling command): `MoveBlock` for each member into the new tab, with compensation that undoes the ones already moved if one fails (the saga's compensation today deletes the new workspace after step 3 fails, which assumes one block).
3. **New tab's layout:** `setup_torn_off_block_layout` seeds one leaf with `blockStack` and `activeBlockId` set, not just `blockId`.
4. **Source removal:** for a header drag, remove the whole leaf as a move (`removeLeafEmptiedByMove`), not one member; and on srv side take every member out of the source tab's layout, not one (`queue_source_layout_delete` is per block today).
5. **Mother window:** with the pane leaving, the existing `measureMotherResize` is right on Windows and Linux; the pill path's `wasOnlyTab` guard stays for pills.
6. **Floating window:** stop assuming `blockids[0]` is the pane (`floating-pane-workspace.tsx` sites above); the floater's close-on-empty must wait for the whole stack; the snapshot and size can stay the visible block's and the pane's.
7. **Redock:** `RedockFloatingPane` (and the cross-window drop overlay) must move every member back and re-create the stack, so a torn-off pane can be put back as it was. Today it moves one block.
8. **Rollback** (`open_floating_pane_window` failing after the move): the pill path's `rollBackTearOff` redocks one block; it needs the whole stack.

Smaller alternatives do not work. Changing only the payload would still move one block in the saga. Moving the other members in a second call after the first would leave the floater with one tab for a moment and race the source's queued layout delete (the race #3708 removed).

## 6. Tests to write with the fix

- Unit: the monitor's payload builder returns all members in order for a stacked node, and one member for a plain node; the active member is the visible one.
- Saga: a three-member tear-off moves all three; a failure on the second member compensates the first.
- Frontend: after a whole-pane tear-off, the source layout has no leaf for the pane (and for a pane that was the window's only pane, the tab auto-closes as today).
- Floater: opens with all members; closing the last member closes the window; redock restores the stack in the same order with the same visible tab.
- Manual, per platform (this RCA does not cover Windows or Linux by running them): a multi-tab pane with an agent, a terminal and a file pane; header-drag it out; check all three survive with their state, then redock.

## 7. Out of scope here

- Dropping a whole pane onto another AgentMux window (`performCrossWindowDrop` is an empty stub in the monitors; the drop overlay handles it with `RedockFloatingPane`, which has the same one-block limit).
- Dragging a pane inside its window: the whole leaf, stack included, moves, so that case is correct.
- Tearing off a whole window tab (`TearOffTab`): it moves the tab with all its blocks and is correct.

## 8. The fix

One change on all three platforms; the header drag now moves the whole stack.

- **Payload:** `pane-tearoff.ts` (`paneTearOffOf`) reads the dragged node's stack once; the three monitors pass it to `TearOffBlock` as a new optional fifth argument `members` (visible tab = `blockId`) and drop each member from the source layout afterwards (`removeTornOffPane`). A pill drag is unchanged: it sends no `members`.
- **Saga:** `tear_off_block::run_stack` moves the visible block first (so it is the floater tab's `blockids[0]`, which the floater's maximize and always-on-top state key on), then the rest in order. A failure on a later member moves the earlier ones back before the new workspace is deleted, because deleting a workspace deletes its blocks.
- **Layout:** `setup_torn_off_stack_layout` seeds the floater's one leaf with `block_stack` and `active_block_id`.
- **Source:** every member goes through `queue_source_layout_delete`; the pane goes with the last one. On Windows and Linux the existing mother-window resize is now right, because the pane really leaves.
- **Redock:** `RedockFloatingPane` takes optional `members` and `activeBlockId`. The first member lands as the pane (insert or ghost split, as before); the others land as its tabs in order, and only the active one is made visible. `floater-panes.ts` groups the floater's blocks into panes for the drag redock and the Redock button, which used to redock every block as its own pane.
- **`stackpush`:** the frontend handler now honours the action's `focused` flag instead of always activating, which the redock needs to keep the visible tab.

Not changed: dropping a pane onto another AgentMux window (section 7).

Agent: Maop@starpower
