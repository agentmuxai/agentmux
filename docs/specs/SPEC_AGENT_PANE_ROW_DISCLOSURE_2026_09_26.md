# Spec: One open/closed model for agent-pane rows

**Date:** 2026-09-26
**Status:** active — PR 1 (reader, writer, estimates, canceled thinking) implemented in #3906; PR 2 (remove `section`) in the PR stacked on #3906
**Amended 2026-10-07:** a `jekt_message` is no longer open by default. It closes like a tool call: held open after arriving live until it scrolls off, pinned open by a click, and open by default only when `TIER=sensitive`. `expandedTools` is now `heldOpenNodes`. Operator's request; see `docs/reports/REPORT_JEKT_COLLAPSE_AND_PREVIEW_SKID_2026_10_07.md` §2.1.
**Scope:** `frontend/app/view/agent/` — how every transcript row decides whether
it is open, how the user toggles it, and how the virtualizer estimates it
**Verified against:** `main` @ `ba9abe92f`
**Source:** `docs/reports/REPORT_TOOL_PREVIEW_DRY_AND_ARCHITECTURE_2026_09_26.md`
items B1, B4
**Builds on:** `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md` (which introduced
`currentExpansion`, phase 1 of that spec)
**Companion:** `SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26.md` (supplies each
tool's `presentation`)

---

## 0. TL;DR

"Is this row open?" is answered in **three places that have to agree**, and
"toggle this row" in **two more**:

1. **Each component, by its own rule:**
   - `ToolBlock`: `pinned || autoExpanded() || userHolding()`, or
     `!userCollapsed` for content-first tools.
   - `AgentMessageBlock` / `JektBubble`: a `collapsed` prop.
   - `UserMessageBlock`: `!isStartup || pinned || hovering`.
   - `PersistentShellBlock`: `pinned`.
   - `MarkdownBlock` (canceled thinking): a local signal the rest of the pane
     can't see.
2. **`currentExpansion()`** (`virtualization/expansion-source.ts`), which feeds
   the layout slice (the virtualizer's real open state).
3. **`estimateNode()`** (`virtualization/renderers.ts`), which re-derives the
   same rules again to pick a height for the perf probe.

Toggling happens in `DocumentRow` (the `e` key: `TOGGLEABLE_KINDS`, then pin
for tool/shell and collapse for everything else, with a content-first
exception) and in each component's click handler, which writes one of three
sets in `AgentDocumentView`.

The content-first WebSearch change (#3874) had to change **all five**. This
spec makes one function answer "is it open" and one function perform "toggle
it"; components, the keyboard, the virtualizer and the estimator all call those.

## 1. Today, per row type

| Row | Default | Opened / closed by | State read | Toggle writes | Component-local |
|-----|---------|--------------------|-----------|---------------|-----------------|
| tool (panel) | closed | pin; auto while running / pending approval; held open after a live finish until scrolled off | `pinnedNodes`, `expandedTools`, `status` | `pinnedNodes` | `userHolding` (mouse inside keeps it open) |
| tool (content-first) | open once finished | user collapse | `collapsedNodes` | `collapsedNodes` | — |
| agent_message, jekt_message | open | user collapse | `collapsedNodes` | `collapsedNodes` | — |
| user_message | open; **startup payload closed** | pin (startup only) | `pinnedNodes` | `pinnedNodes` | `hovering` (overlay peek, not in-flow) |
| shell | closed | pin | `pinnedNodes` | `pinnedNodes` | — |
| markdown (canceled thinking) | closed | click | — | — | `expanded` signal. **The virtualizer never learns it opened** (`expansion-source.ts` header documents the gap) |
| section | open | click | `node.collapsed` (`currentExpansion`) | `collapsedNodes` (`DocumentRow`) | — |
| everything else | open, fixed | never | — | — | — |

Two defects fall out of that table:

- **Sections read one field and write another.** More importantly, **nothing
  produces a `section` node**: there's no `type: "section"` constructor anywhere
  outside `types.ts`. The kind, its renderer branch, its toggle and its estimates
  are dead code.
- **Canceled-thinking expansion is invisible to layout.** Expanding one changes
  its height without the layout slice knowing it's open, so the slice keeps the
  collapsed estimate until the measure pass corrects it.

## 2. Design

### 2.1 One reader

```ts
// virtualization/disclosure.ts (new)
interface RowFlags { pinned?: boolean; collapsed?: boolean; held?: boolean }
interface Disclosure {
    open: boolean;
    /** Why it's open: the layout slice's Expansion.via ("default" when closed). */
    via: "default" | "auto" | "pin";
    /** What a header click or the `e` key flips: pinnedNodes, collapsedNodes, or nothing. */
    toggle: "pin" | "collapse" | null;
}
function rowDisclosure(node: DocumentNode, flags: RowFlags): Disclosure;
function rowDisclosureIn(node: DocumentNode, state: DisclosureInputs): Disclosure; // flags from the sets
```

- `rowDisclosure` holds the whole of §1's table in one `switch`. Tool
  presentation comes from the descriptor (`isContentFirstTool`), and tool
  status from `TOOL_STATUS` (a `dismissed` tool never holds open).
- It takes a node and its **flags**, not the whole state, so each component
  calls it with the flags it already receives as props (`pinned`,
  `heldOpen`, `userCollapsed`), and no component keeps its own rule. The
  prop names and the ~40 existing ToolBlock tests stay as they are. The
  virtualizer calls `rowDisclosureIn`, which derives the flags from the sets.
- `currentExpansion()` becomes a two-line adapter over it (the layout slice's
  `Expansion` shape is unchanged).

### 2.2 One writer

- `toggle` in the result says which set the row's toggle flips: `pinnedNodes`
  when the default is closed, `collapsedNodes` when it's open, or nothing.
  `ToolBlock`'s header click and `DocumentRow`'s `e` key both route through it
  (to the existing `onTogglePin` / `onToggleCollapse` callbacks), which
  replaces `TOGGLEABLE_KINDS` and the tool/shell special case.
- `toggleDisclosure(node, state)` is the same thing as a pure state transform,
  for tests and any future caller that holds the state directly.
- Effect on the `e` key: it now also toggles a startup payload and a canceled
  thought (both already click-toggleable), and no longer targets the dead
  `section` kind. Normal user input stays non-toggleable, per #1020's intent.

### 2.3 Storage stays

The three sets keep their names and meaning:
- `pinnedNodes`: the user opened a closed-by-default row.
- `collapsedNodes`: the user closed an open-by-default row.
- `expandedTools`: the finish hold.

Merging them into one set would be cleaner, but they are:
- **persisted** (`useSnapshotPersistence.ts` writes `collapsedNodeIds` and
  `pinnedNodeIds`, which `useHistoryPagination.ts` reads back);
- **used for roll-off**, which keeps pinned rows as `keepIds`
  (`agent-view.tsx:976`);
- **pruned together** (`agent-view.tsx:1006`).

A merge would need a snapshot migration for no user-visible gain. With reads
and writes centralized, the storage becomes an internal detail of
`disclosure.ts`, and a later merge would touch only that file plus the
migration.

### 2.4 Estimates derive from the reader (report B4)

`estimateNode(node, state)` becomes
`estimateNodeForState(node, rowDisclosure(node, state).open ? "expanded" : "collapsed")`.
The per-kind open/closed checks inside `estimateTool`, `estimateAgentMessage`,
`estimateJektMessage`, `estimateUserMessage` and `estimateShell` go away. The
perf probe then measures the estimate the layout actually uses.

The content-first tool's expanded estimate moves into `estimateNodeForState`'s
expanded branch, which the layout slice uses. Today it's only in
`estimateTool`, which the slice never calls, so the slice still pre-sizes a
history WebSearch at the 200 px generic tool estimate.

### 2.5 The component-local cases

**Found by the parity test and fixed:** a canceled or denied tool enters
`expandedTools` on its active → inactive transition. `ToolBlock` rendered it
closed (a dismissed tool skips the hold), but `currentExpansion` reported it
open, so the layout slice sized it as expanded. The shared rule applies the
dismissed check for both.


- **`ToolBlock.userHolding`** (keeps an auto-open tool open while the mouse is
  inside it, so a scroll-off release can't fold it mid-read) stays local. It's
  a transient override on top of `open`, not a state the virtualizer needs:
  the row doesn't change height while held.
- **`UserMessageBlock.hovering`** stays local. It opens a Portal overlay, not
  in-flow content, so layout is unaffected.
- **Canceled-thinking `MarkdownBlock`** moves into the model: it's
  closed-by-default and toggleable, so expanding it writes `pinnedNodes`
  through `toggleRow`. This closes the layout gap from §1, and roll-off keeps
  an expanded canceled thought like any pinned row.

### 2.6 Remove the dead `section` kind

`SectionNode` has no producer. Remove the type, the renderer branch in
`DocumentRow`, its `currentExpansion` / estimator / `STREAMING_CAPABLE` /
`btw.ts` cases, and `TOGGLEABLE_KINDS`' entry. A `never` exhaustiveness check
already guards the switch statements, so the compiler finds every site.

## 3. Non-goals

- No behavior change for any row that renders today, apart from the §2.5
  canceled-thinking layout fix, which is invisible except for the removed
  one-frame height correction.
- The three state sets and the snapshot format stay as they are.
- The layout slice's reducer and `Expansion` type stay as they are.

## 4. Phasing

1. **PR 1: reader and writer.** Adds `disclosure.ts`, turns
   `currentExpansion` into an adapter, and adds `toggleRow`. Components move to
   `open` / `onToggle`. The `e` key and the estimators derive from the reader.
   Canceled thinking moves into the model.
2. **PR 2: remove the `section` kind.** Kept separate so the deletion reviews
   on its own.

## 5. Tests

- **Parity, the core test.** For every row type and every relevant state (a
  table of about 30 cases: each tool status × pinned/held/collapsed,
  content-first vs panel, startup vs normal user message, agent/jekt collapsed
  or not, shell pinned or not, canceled thinking expanded or not):
  - Render `DocumentRow` and assert that its rendered open state (the panel
    class, the body `<Show>`) equals `rowDisclosure(node, state).open`.
  - Assert that `currentExpansion` agrees.
  - Assert that `estimateNode` equals
    `estimateNodeForState(node, open ? "expanded" : "collapsed")`.
- **`toggleDisclosure`:** each toggleable type flips exactly one set and
  round-trips back; non-toggleable types return the same state object.
- **Keyboard:** `e` on each toggleable row flips it; on others, it does nothing.
- **Canceled thinking:** expanding it resolves `currentExpansion` to open, so
  the layout slice receives `ExpansionResolved`.
- **The existing suites** pass: `expansion-source.test.ts`,
  `renderers.test.ts`, `ToolBlock.test.tsx`, the `AgentDocumentVirtualList.*`
  collapse / pin / handoff tests, and `DocumentRow.test.tsx`. So do `tsc` and a
  live dev-build check toggling each row type.

## 6. Acceptance

- One function answers "is this row open" (`rowDisclosure`), and its
  `toggle` field says what a toggle flips. No component, the keyboard, the
  layout slice or the estimator keeps its own copy of either rule.
- `estimateNode` contains no open/closed logic.
- `TOGGLEABLE_KINDS`, `ToolBlock`'s `autoExpanded` / content-first branch, and
  `MarkdownBlock`'s local expand signal no longer exist. (The `onTogglePin` /
  `onToggleCollapse` callbacks stay: they're the two storage sets' writers,
  and the rule picks between them.)
- No `section` node type (PR 2).
