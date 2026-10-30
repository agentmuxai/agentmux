# Two Editor panes that end up as tabs of one pane merge into one Editor

**Status:** active — built in PR #5952 (task #11410), except the bubble animation (§5) and the exact scroll offset (§3); the operator approved all seven §7 recommendations on 2026-10-30. Where the build differs from the text below: a second, different unsaved copy opens as an untitled tab with a notice (the Editor has no way yet to name a scratch tab), and a mid-save source stops the merge with a notice instead of retrying.
**Author:** Camper@narko, at the operator's request (2026-10-30).
**Extends:** [SPEC_MINIMIZE_MERGES_INTO_ADJACENT_MINIMIZED_PANE_2026_10_28.md](SPEC_MINIMIZE_MERGES_INTO_ADJACENT_MINIMIZED_PANE_2026_10_28.md) and [SPEC_MINIMIZE_MERGES_INTO_EXPANDED_NEIGHBOUR_2026_10_30.md](SPEC_MINIMIZE_MERGES_INTO_EXPANDED_NEIGHBOUR_2026_10_30.md) (the same "bubble into" idea, one level down). Builds on [SPEC_DOCUMENT_TABS_2026_10_02.md](SPEC_DOCUMENT_TABS_2026_10_02.md) §5.8 (moving a document tab between panes) and §6.
**Scope:** `layoutStack.ts` (the stack mutators every route uses), `doc-tab-hosts.ts` (`moveDocTab`), `editor-doc-tabs.ts` (the Editor's host), `PaneChrome.tsx` (drop handlers), `layoutMinimizeMergeApply.ts`.

The operator's words (10-30): *"lets also refine pane dynamics a bit, its an extension of the adjacent minimized panes collapse their pills. In this case, if two document panes tabs are open in the same pane, they collapse together, with the document tabs from both consolidating over the destination pane."*

## 0. Reading, and the other reading

**Reading (AgentY's, which I share):** whenever an Editor pane tab lands in a pane that already holds an Editor pane tab, the two do not sit side by side as two pane tabs. They become **one Editor**: the destination keeps its pane tab and its document tabs, the arriving Editor's **document tabs are appended** to the destination's strip, and the arriving pane tab goes. Two Editors in **separate tiles** stay separate: this is about two Editors *in the same pane*.
**Other reading:** only the minimize merge does it, and a tab-pill drop keeps two Editor pane tabs. Not recommended: the same state (two Editors in one pane) would then be reachable by one gesture and avoided by another.

A document tab (a file in the Editor's own strip) is the third layer of tab, below window tabs and pane tabs. Two Editor pane tabs in one pane are two document strips one click apart, with the same files open twice and nothing joining them: the merge removes that.

## 1. Every route by which two Editor pane tabs end up in one pane

All of these pass through a few stack helpers in `layoutStack.ts`, so the merge belongs there (§6), not in each caller.

| # | Route | Today | With the merge |
|---|---|---|---|
| 1 | **Tab-pill drag**: a pill dropped on another pane's strip or header (`handleReceiveForeignTab(s)` → `moveBlockInStack` / `moveBlocksInStack`), incl. #10591's drop on a header or body | two Editor pane tabs | merges |
| 2 | **Minimize merge** into a minimized neighbour, and into an expanded one (the amendment): `moveBlocksInStack` | two Editor pane tabs | merges |
| 3 | **Pane "+" picker** adds an Editor to a pane that already has one (`addWidgetAsPaneTab`) | a second, empty Editor pane tab | no second Editor: opens a **new document tab** in the existing one (as Ctrl+T does) |
| 4 | **Multi-select move** of several pills (`moveBlocksInStack` with a selection) | as 1 | merges each arriving Editor, in order |
| 5 | **An agent's** `Pane` move / open into a stack, another window's drop, redock from a floating pane (srv `MoveMember` actions, `layoutPersistence.ts`) | two Editor pane tabs | does **not** merge on its own (§6); the pane-tab menu offers **"Merge document tabs"** |
| 6 | **A saved layout** that has two Editor members in one stack | two Editor pane tabs | not merged at load (a load never rearranges the person's tabs, as in the minimize spec §2.3); the same menu item merges |

Not a route: Hangar's and an agent's *open file* (`OpenEditor`, `OpenFiles`) add a document tab to an existing Editor already (SPEC_DOCUMENT_TABS §5.7), and two Editors in two tiles are not in one pane.

## 2. Which views are "document panes"

- **Editor: yes.** The view that has document tabs. **Media: yes, the same view**: `media` is an alias of the Editor (`editor.tsx`, `media-migration.ts`), so a Media pane is an Editor pane and merges like one.
- **Hangar (files): no.** A folder is a place, not a document (document tabs were withdrawn for it, SPEC_DOCUMENT_TABS §6.2): its folder tabs are pane tabs and stay so.
- **Browser, Terminal, Agent, Swarm, Memory, sandboxed widgets: no.** One page / session / list per pane (§6.4).
- **Only like with like:** the two hosts must have the same `docType` **and the same scope** (an Editor's connection: this computer, or one remote). An Editor on a remote and one here stay two pane tabs, because a path means something different on each (`moveDocTab` already refuses this, "Its file is on another computer").

## 3. What carries over

The Editor's document host already moves a document tab with its live state, for a drag of one document tab between Editors (`DocTabHost.give` / `take`, SPEC_DOCUMENT_TABS §5.8). The merge uses it for every document tab of the arriving Editor, so **it carries what a drag of one tab carries**:
- **Carries:** the open documents, in their order and their pin; the **text including unsaved edits** and the dirty mark; the encoding a save writes back; the tab's mode; the undo history and the selection (so the caret is where it was). Scratch (unsaved new) documents carry with their identity.
- **Order and active:** the destination's tabs first, unchanged; the arriving ones after them in their order. The **destination's active document stays active**; the arriving Editor's active document is the one *next to it*, so the person sees nothing jump. The merge's last arriving tab gets the strip reveal (the earlier merge's scroll to the end).
- **Scroll:** the carried selection restores the view to the caret; the exact scroll offset is not carried today. The build checks whether adding it to the transfer is cheap; if not, scrolling to the caret is the answer.
- **File tree** (shown in the pane, its root and open folders): the **destination's** stays; the arriving one is dropped. It is a view of the pane, not of a document.
- **Preview tab:** at most one per pane. If both have one, the destination's is kept as the preview and the arriving one becomes a normal tab (nothing is lost).
- **Pane-level meta** of the arriving block (title, colour, follow-folder): not carried; the destination's stays.
- **Conflicts (the same file open in both).** Opening the same key twice cannot stay (a key is one tab). Rule, in order:
  1. **Both clean:** one tab; the destination's stays as it is and the arriving copy is dropped (the file is the same).
  2. **One dirty, one clean:** the **dirty** copy wins: its text and history replace the clean tab's, in the destination's tab slot. No unsaved text is ever dropped.
  3. **Both dirty, texts differ:** the destination's stays; the arriving unsaved text becomes a **scratch tab "name (from the merged Editor)"**, dirty, next to it. Nothing is lost and no prompt is asked; the person compares or saves.
  4. **Both dirty, same text:** one tab.
- **A document that is mid-save** in the arriving Editor refuses give (`refuseGive`): the merge waits for the save (a short retry), then runs; if it cannot, the two stay as separate pane tabs and a quiet notice says why.
- The arriving Editor's **pane tab is then closed** as a move, not a delete of work: its documents are already in the destination, so it has nothing left that a close would ask about. No dirty prompt.

## 4. Undo

There is **no undo of the merge itself** (as the minimize merges, which chose the same). A document tab goes back to its own pane through the Editor's existing **"Move to new pane"** on a document tab's menu, or by dragging it to a split. *Alternative (decision 4):* a "Merged 3 documents into <Editor>. Undo" notice for ~8 s that moves exactly those tabs (by id) into a fresh Editor pane tab beside the destination.

## 5. The animation

The same bubble as the minimize merge and the pill merge (#8675 / `mergeInto`): each of the arriving Editor's **document tabs travels from where it was drawn** (its own document strip if that pane was on screen, else from the pane-tab pill that landed) **into the destination's document strip** and lands with the landing bounce. Category `transition`, the app's motion policy, with motion off the tabs appear at once. The pane tab's own pill is the one animation of #8675/#10591; the document tabs' flight starts when it lands, and the two do not overlap.

## 6. Where it lives, and with the tab-pill drag (Camper-2, #10591)

A single hook, `mergeArrivingDocumentPanes(model, targetBlockId, arrivingBlockIds)`, **registered into `layoutStack.ts`** the way `setMinimizeMerger` is (the layout module may not import the Editor), and called from `moveBlockInStack` / `moveBlocksInStack` after a move **across** stacks for a person's act. So #10591's two drops (header and body), the minimize merges and the multi-select move all merge without touching their own code. Order: the hook runs **before** the layout commit announces the move, so the arriving Editor is merged and closed in the same edit and no frame shows two Editor pane tabs. #10591 only has to call the helper it already calls; I will tell Camper-2 this seam when the spec is approved, and the amendment's expanded-neighbour merge uses it unchanged.

Agent and srv-originated moves, redock and loads do not call the hook (§1 rows 5 and 6): the arriving block there may be in another window that holds its live state, and a load or an agent should not rearrange documents on its own. The "Merge document tabs" menu item runs the same hook by hand.

## 7. Decisions (each with my recommendation)

1. **Merge on every person's route, not only minimize?** *Yes* (the reading in §0).
2. **Agent / load / redock routes merge automatically?** *No:* only the person's acts; a menu item for the rest (§1 rows 5 and 6).
3. **Conflict policy** (§3): *as listed:* a dirty copy always survives, two different dirty copies become a second tab, never a prompt.
4. **Undo:** *none, documents move back with "Move to new pane"* (consistent with the earlier merges); the 8 s Undo notice is the alternative if the operator wants it, at the cost of recreating a pane.
5. **Opt-out:** *hold **Alt** while dropping a pill* keeps the two as separate pane tabs, and a pane-tab menu item **"Keep as separate Editor"** is not needed (Move to new pane covers it). Minimize has no opt-out. One way only.
6. **Remote Editors** of different connections stay separate; *recommend that,* since their files are not each other's.
7. **The "+" picker** on a pane that has an Editor opens a new document tab instead of a second Editor pane tab. *Recommend that,* so the merged state is the only state.

## 8. Tests and recording (when built)

- **Unit:** the merge helper over fake hosts: order, active, destination's active unchanged; each conflict rule (clean/clean, dirty/clean, dirty/dirty different, dirty/dirty same); preview collapse; a mid-save refusal leaves both pane tabs; different scopes and non-document views are untouched; the closed arriving pane tab asks nothing and deletes no live document; the hook runs for a person's drop and a minimize merge and not for an agent move or a load; Alt on a drop keeps two pane tabs; the "+" picker opens a document tab.
- **Component:** the destination's strip shows the arrived document tabs with the landing class; the "Merge document tabs" menu item appears only for a stack with two Editors.
- **Live** in my own task dev build: two Editors with files and an unsaved edit each, a pill drop, a minimize merge; the same file open in both (the four conflict cases); a recording of the bubble.

## 9. Not in scope

Merging Editors in separate tiles; merging non-Editor types; changing how a drag of one document tab works; a cross-window merge.
