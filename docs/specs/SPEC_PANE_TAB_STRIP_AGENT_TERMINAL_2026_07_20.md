# SPEC: Pane tab strip — editor-style in-pane tabs for agent and terminal panes

**Date:** 2026-07-20
**Status:** implemented — shared PaneTabStrip across editor/agent/terminal via PRs #2250/#2254/#2261/#2282; verified in code 2026-08-10.
**Scope:** Agent pane (`frontend/app/view/agent/**`), terminal pane (`frontend/app/view/term/**`),
layout store, agent runtime, shell controller
**Author:** Agent3
**Related:**
`docs/specs/SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md` (the fork model + block-stack
mechanism this spec builds on, and whose §7 fork-bar UI this spec supersedes),
`docs/specs/SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md` (the definition-fork backend
mechanism, reused unchanged),
`docs/specs/SPEC_EDITOR_TABS_2026-05-26.md` (the editor's existing multi-tab system — the
visual/interaction reference this spec generalizes),
`frontend/app/view/editor/editor-tab-strip.tsx` / `editor-view.scss` (the concrete component
and styles being lifted out and reused)

> **Naming note, stated precisely up front.** This feature is **in-pane tabs** — multiple
> tabs *within* one agent or terminal pane, exactly like the editor's existing tab strip. It is
> **not** what this codebase already calls **"inter-pane"** (peer-to-peer MCP messaging
> *between* agent panes — `send_message`, `inject_terminal`, `broadcast_message`; see
> `docs/specs/openclaw-widget.md`, `docs/specs/integration-vision.md`,
> `docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md`). It is also not the outer app's top-level
> browser-style tab strip (`workspace.tsx`, covered by
> `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md`) — that holds whole workspaces of panes, a
> different layer entirely. This doc's title says "pane tab strip," not "inter-pane tabs,"
> specifically to not collide with the already-taken term.

---

## 1. Intent

The user's ask, stated directly: **give agent panes and terminal panes the same in-pane tab
strip the editor already has** — same visual chrome, same interaction model (click to
activate, hover to reveal close ×, middle-click to close, tooltip on hover) — **plus a
persistent `+` button pinned to the far right of the strip**, so a user always has an obvious,
discoverable way to add another tab. This is explicitly **part of the forks feature**: for
agent panes, a "tab" *is* a fork (§6 of `SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md`,
unchanged). For terminal panes — which have no such concept today — this spec proposes the
parallel primitive.

## 2. Current state

### 2.1 The editor — the only pane type with real in-pane tabs today

`frontend/app/view/editor/editor-tab-strip.tsx` renders `.editor-tab-strip`: a flex row of
`.editor-tab` pills (28px tall, one per open file), each with a label, a hover-revealed ×
close button, and a `Tooltip` showing the full path. Active tab gets an accent underline
(`box-shadow: inset 0 -2px 0 0 var(--accent-color)`) plus a lighter background; dirty tabs
always show their ×; preview tabs (VS Code-style, replaced by the next single-click) render
italic. State lives in `frontend/app/store/editor-pane-state-store.ts` (slice #10) — a pure
`{tabs[], activeTabId, recentlyClosed[]}` reducer, dispatched via `OpenFile`/`CloseTab`/
`SwitchTab`/etc. There is **no `+` button today** — new editor tabs are opened by clicking a
file in the tree, not by an explicit "add tab" affordance. (§7 proposes adding one, for
consistency with the new agent/terminal strips this spec introduces.)

### 2.2 Agent panes — the fork model is designed and partially built, but unwired

`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md` already designed this thoroughly:

- **A fork is just a `blockId`** — its own `AgentInstance`, its own controller, its own
  `agent:<defId>:current` transcript zone. A pane hosts an ordered **fork set**, one active.
  This is the "reuse the runtime" decision (§8 of that spec) — deliberately **not** a
  multi-session-per-block runtime change.
- **The fork mechanism is validated and working**: `ForkAgentDefinitionCommand` (from
  `SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md`) creates a new `AgentDefinition` with
  `parent_id`/`branch_label`; spawning it with `--resume <parentSid> --fork-session` shares
  history up to the fork point and diverges with a fresh session id. Confirmed empirically
  2026-06-15 (§6.4 of the June spec) — this part needs no further validation.
- **The one new structural piece — a layout-node `blockStack`/`activeBlockId`** — is
  specified (§6.2) but **not yet implemented**: no `blockStack` field exists anywhere in
  `frontend/app/store/` today (confirmed by search). This is still the single missing piece
  that turns "a fork is a block" into "a pane can show one of several forks."
- **Four Phase 1/2 PRs already landed on `main`**, ahead of the layout-store piece:
  `frontend/app/view/agent/components/PaneRegions.tsx`, `PaneRow.tsx` (+ `.scss`/tests), and
  `frontend/app/view/agent/fork/{fork-set.ts, useForkSet.ts, ForkBar.tsx}` (+ `.scss`/tests).
  `computeForkSet()` (`fork-set.ts:78`) is a solid, already-tested pure derivation (walks
  `parent_id` lineage to the root, BFS-orders descendants) — reusable as-is. `ForkBar.tsx`
  renders each fork as a `<PaneRow>` (§5.2 of the June spec's shared pin-row chrome) with a
  `+ fork` text button already positioned last in the row list (`ForkBar.tsx:67-77`) — i.e.
  the *far-right placement* this spec asks for already exists structurally, just styled as a
  `PaneRow`-family button rather than an editor-tab-strip `+`.
- **But none of it is wired into `agent-view.tsx`.** Neither `PaneRegions` nor `ForkBar` is
  imported/rendered anywhere in the live agent pane tree (confirmed by grep — zero hits).
  Today, opening the same agent definition twice still opens a **separate top-level pane**,
  not a fork inside one pane. This is orphaned, tested, working code sitting unused.

### 2.3 Terminal panes — clean slate, and the obvious "just reuse ShellNode" shortcut doesn't work

- **One term pane = one block = one `ShellController` = one PTY, no exceptions.**
  `agentmux-srv/src/backend/blockcontroller/shell/controller.rs` holds exactly one
  `conn_name`/`input_tx`/`child_pid` per block (`ShellControllerInner`, `controller.rs:45-72`);
  `lifecycle.rs:149` opens exactly one `portable_pty` on `start()`. No array/map of processes
  anywhere in this controller. `frontend/app/view/term/term.tsx` (`TerminalView`) and
  `TermViewModel` likewise own exactly one xterm instance per block. There is no split,
  multiplex, or session-index concept anywhere in this stack today.
- **The existing `ShellSessionRegistry` (`agentmux-srv/src/backend/shell_node.rs`) is a
  tempting but wrong shortcut.** This is the "persistent shell" primitive that lets an
  *agent* pane pin background shell processes into its `ActivityDock` (one row per
  `ShellNode`, spec: `SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md`). It looks like "N shell
  sessions tracked by one pane" — but it's the wrong shape for terminal-pane tabs on three
  counts: **(a)** it's pipe-based (`tokio::process::Command`), not a real PTY — no resize, no
  job control, no curses apps, and the module's own comment flags PTY support as an
  unimplemented "Phase 3 follow-up"; **(b)** every `ShellNode` requires an owning
  `agent_block_id` (`shell_node.rs`, `server/mod.rs:563,614,646`) for cwd/env fallback and WPS
  scoping — there's no standalone spawn path, so it can't back a plain terminal pane that has
  no owning agent; **(c)** persistence is a capped in-memory ring
  (`MAX_EXITED_STATUS = 512`, `shell_node.rs:59`), not the wstore-backed `Block` persistence a
  real tab needs to survive a reload. **Do not build terminal-pane tabs on this registry** —
  it's a useful pattern reference (per-session registry, stop/status/stdin-relay), not a
  reusable backend.
- **The good news: the layout mechanism agent forks need is view-agnostic, so terminal panes
  get it for free.** `LayoutNode`/`LayoutNodeData` (`agentmux-common/src/layout_types.rs:32-75`)
  is a bare `{ id, flexDirection, size, children, data: { blockId } }` — no `view` field; view
  lives on `Block.meta["view"]`, resolved at render time, not on the node. **The
  `blockStack`/`activeBlockId` extension §6.2 of the June spec proposes for agent panes is not
  agent-specific at all** — it can be added to `LayoutNode` once and apply uniformly to any
  pane type, agent or terminal.
- **The RPC shape for "create another block in this pane" already has a precedent to copy.**
  `open_pane_floating` (`agentmux-srv/src/server/app_api/pane.rs:30-56`) already creates a
  `Block` via the reducer's `Command::CreateBlock` **without** placing it in the tile layout
  ("we skip layout placement," per its own comment). A terminal "+ new tab" action is
  structurally the same move: create a shell `Block` + `ShellController`, but push its id onto
  the *pane's existing* `blockStack` instead of creating a new `LayoutNode`/tile.

---

## 3. Design: one shared `<PaneTabStrip>`, editor-style, everywhere

### 3.1 Extract, don't reimplement

Lift `editor-tab-strip.tsx`'s visual/interaction contract into a new, generic,
pane-type-agnostic component:

```
frontend/app/view/shared/PaneTabStrip.tsx
frontend/app/view/shared/_pane-tab-strip.scss
```

Generic over a minimal tab shape — deliberately smaller than `EditorTab` (no `filePath`,
`isPreview`, `contentHash`, etc. — those are editor-specific):

```ts
export interface PaneTabStripTab {
    id: string;
    label: string;
    /** Full-text tooltip content; defaults to `label` when omitted. */
    tooltip?: string;
    /** Dirty-equivalent — e.g. an agent fork mid-turn, a shell with unread output.
     *  Same visual contract as the editor's dirty tab: close × always visible. */
    attention?: boolean;
    /** Status accent dot, reusing PaneRow's accent vocabulary where it fits
     *  ("running" | "idle" | "error" | ...) — optional, tabs work without one. */
    accent?: "running" | "idle" | "error" | "done" | "neutral";
}

export interface PaneTabStripProps {
    tabs: PaneTabStripTab[];
    activeId: string | null;
    onActivate: (id: string) => void;
    onClose?: (id: string) => void;
    /** The far-right `+` — omitted entirely (not just disabled) when the pane
     *  type has no "add tab" action yet. */
    onAdd?: () => void;
    addTitle?: string; // tooltip for the + button, e.g. "New fork" / "New shell tab"
}
```

Behavior ported verbatim from `editor-tab-strip.tsx`: click-to-activate, middle-click-to-close,
hover-reveals-×, `attention` tabs always show ×, `Tooltip` (Portal-based, not native `title` —
the strip clips overflow so a CSS tooltip would be cut off) on hover. **New behavior this spec
adds:** a `.pane-tab-strip-add` button, `flex: 0 0 auto`, pinned as the strip's last flex
child — always at the visual far right regardless of tab count or strip scroll state — shown
only when `onAdd` is provided.

Styling: promote `.editor-tab`/`.editor-tab-strip`'s rules from `editor-view.scss` into the new
shared `_pane-tab-strip.scss` as `.pane-tab`/`.pane-tab-strip` (same box model, same
`color-mix`/`var(--accent-color)` tokens, same 28px height, same active-underline
`box-shadow`), then have the editor's own strip consume the shared component instead of its
private markup — so there is **one** implementation, not three copies that drift. This is a
low-risk, high-value refactor: the editor's tests (`editor-tab-strip` currently has none as a
standalone file — behavior is exercised via `editor-pane-state-store.test.ts`) aren't
disturbed, since only the rendering shell moves, not the state logic.

### 3.2 The `+` button, and retrofitting it onto the editor

Add `onAdd`/`addTitle` support to the shared component (§3.1) and wire it three ways:

- **Editor:** `+` → same action as today's "New scratch buffer" affordance
  (`EditorViewModel.openScratch()` already exists) — the editor gains the explicit,
  discoverable "+" it currently lacks, for free, as a side effect of the shared component.
- **Agent pane:** `+` → the fork action (§4.1) — this is exactly `ForkBar.tsx`'s existing
  `onFork` prop, just fired from the new strip's `+` instead of the old `+ fork` text button.
- **Terminal pane:** `+` → the new-shell-tab action (§4.2).

### 3.3 Placement: top of the pane, not the June spec's bottom region — an explicit deviation

The June fork spec placed the fork bar in a new **bottom** region, below the composer
(§7, mirroring Claude Code's `↓`-to-resume gesture and staying near the input focus point).
This spec **changes that placement to the top of the pane**, directly under the pane's
header/toolbar row — matching the editor (tabs sit above the content) and every general
tabbed-UI convention (browser tabs, VS Code, IDE tabs) the user is explicitly asking to mirror.

Rationale for the deviation:
- The whole point of asking for "the same tab style the editor uses" is visual/positional
  consistency across all three pane types — a bottom-anchored strip for agent/terminal next to
  a top-anchored strip for editor would look like two different features, not one pattern.
- A persistent `+` is far more discoverable in the position users already scan for "add" in
  every tabbed app they know (top-right of the tab row) than tucked into a bottom strip below
  a text composer.
- The June spec's `↓`/`↑` keyboard-cycle gesture (§6.5) is a **keybinding**, not a
  mouse-position dependency — it keeps working identically regardless of where the strip
  renders visually. Moving the strip to the top costs nothing functionally.

This is the one explicit design change from the June draft; §9 (Open questions) flags it for
sign-off rather than treating it as silently decided, since the June spec's authors had a
real (if now superseded) rationale for the bottom placement.

For the agent pane, this means retiring `PaneRegions`' proposed bottom `forks` region in favor
of a `tabs` region at the very top (above `top-fixed`), and for the terminal pane, introducing
the pane's first-ever region concept (today `term.tsx` has no region model at all — the strip
is simply the first child, conditionally rendered).

---

## 4. Backend / data model

### 4.1 Agent pane forks — unchanged from the June spec

No change to the mechanism. Reuse verbatim:
- Fork = `ForkAgentDefinitionCommand` → `AgentInstance` with `parent_instance_id` →
  spawn with `--resume <parentSid> --fork-session` (§6.3 of the June spec; validated §6.4).
- **The only missing piece is `blockStack`/`activeBlockId`** on the layout node (§6.2) — this
  spec's layout-store work (§4.3) is what the June spec's Phase 2 already called for; it just
  hadn't landed yet.
- `computeForkSet()` (`fork-set.ts`) is reused as-is to derive the tab list; a `ForkSetEntry`
  maps to a `PaneTabStripTab` with `label: entry.title`, `accent: entry.isActive ? "active-ish
  mapping" : entry.blockId ? "running" : "idle"` (same accent logic `ForkBar.tsx:34-37`
  already has, just feeding the new strip instead of `PaneRow`).
- Dormant-fork lifecycle (keep-alive vs. suspend-on-blur, §6.6 of the June spec) is unchanged
  and still an open call — this spec doesn't need to resolve it to ship the UI layer.

### 4.2 Terminal pane tabs — new primitive, modeled on the agent fork's shape but simpler

A terminal tab has no forking/lineage/session-inheritance concept — it's just "another shell."
Proposed shape:

- **New RPC, `newpanetab`** (name tentative): `{ pane_layout_node_id, cwd?: string }` →
  creates a `Block` with `meta.view = "term"` via the reducer's existing `Command::CreateBlock`
  (same primitive `open_pane_floating` already uses to create a block without layout
  placement), spawns its `ShellController`/PTY normally (unchanged —
  `blockcontroller/shell/lifecycle.rs`), and returns the new `blockId` **without** touching the
  tile layout. The caller pushes the returned id onto the pane's `blockStack`.
- **No new backend concept for "a shell tab"** beyond one more ordinary shell `Block` — the
  entire feature is: (a) don't place it in the layout, (b) track it in the owning pane's
  `blockStack` instead. Every existing shell-controller mechanism (resize, PTY I/O, exit
  handling, `ShellController` lifecycle) is reused untouched, exactly as the agent side reuses
  its untouched controller/transcript machinery.
- **Labeling:** default tab label = the shell's cwd basename or a running counter ("Terminal",
  "Terminal 2", …), matching common terminal-app convention; a later phase can add rename.
- **Closing a tab** = normal block teardown (kill the PTY, same path a pane-close already
  uses) + pop from `blockStack`; closing the last/only tab closes the pane, same as today.

### 4.3 One shared layout-store change serves both pane types

Extend `LayoutNodeData` (or the leaf's block-holding structure) with:

```ts
blockStack: string[];     // ordered blockIds; defaults to [existing blockId] — 100% back-compat
activeBlockId: string;    // must be a member of blockStack
```

Confirmed view-agnostic (§2.3) — this is **one** change in the layout store + tile renderer
(render `activeBlockId`'s block; keep the rest of the stack's blocks alive/tracked), consumed
by both the agent fork-switch flow and the terminal tab-switch flow identically. Stack
membership + active id persist with the rest of layout state, same durability as today's panes
(a pane reopens with its tabs intact) — no new persistence code, per the June spec's approach.

---

## 5. What "the same tab style" concretely buys, side by side

| | Editor (today) | Agent pane (this spec) | Terminal pane (this spec) |
|---|---|---|---|
| A "tab" is | an open file | a fork (`blockId`) | a shell session (`blockId`) |
| Backend primitive | `readeditorfile`/`writeeditorfile` (unchanged) | `ForkAgentDefinitionCommand` + `--fork-session` (unchanged, §4.1) | new shell `Block` via `CreateBlock`, no layout placement (§4.2) |
| Tab-switch mechanism | view-local content cache swap | `blockStack`/`activeBlockId` swap (new, §4.3) | `blockStack`/`activeBlockId` swap (same new mechanism) |
| Visual chrome | `.editor-tab-strip`/`.editor-tab` | `<PaneTabStrip>` (shared, §3.1) | `<PaneTabStrip>` (shared, §3.1) |
| `+` button | **new** (retrofit, §3.2) | replaces `ForkBar`'s `+ fork` text button | **new** |
| Placement | top (unchanged) | top (**moved** from June spec's bottom, §3.3) | top (new) |

---

## 6. Phasing

| Phase | Deliverable | Risk | Notes |
|---|---|---|---|
| **1** | **Extract `<PaneTabStrip>`.** Pull `.editor-tab-strip` markup/styles into `frontend/app/view/shared/PaneTabStrip.tsx` + `_pane-tab-strip.scss`; editor consumes it (pixel-identical behavior, `+` added per §3.2). | low | Pure refactor + one small addition; independently shippable, reviewable alone. |
| **2** | **Layout-store `blockStack`/`activeBlockId`.** The shared mechanism (§4.3), tile renderer support. No UI yet — this is the piece the June spec's Phase 2 called for and that never landed. | med | Touches the layout store + tile/saga paths; needs its own focused review (tear-off interaction, persistence shape). |
| **3** | **Agent fork tabs (read-only switch).** Wire `computeForkSet()` + `<PaneTabStrip>` into `agent-view.tsx`'s top region, consuming Phase 2's stack. Retire the unwired `ForkBar`/`PaneRegions`/`PaneRow` fork-bar path (§2.2) in favor of the new strip — `fork-set.ts`'s derivation logic is kept, its `PaneRow`-based rendering is not. | med | Mostly wiring already-built, already-tested pieces; the delta is the visual layer. |
| **4** | **Agent fork action (`/btw` + strip `+`).** Unchanged from the June spec's Phase 3 — reuse the already-passed validation gate (§6.4 there). | med | No new risk; just hooking the existing, validated mechanism to the new `+`. |
| **5** | **Terminal tabs.** `newpanetab` RPC (§4.2), wire `<PaneTabStrip>` into `term.tsx`'s new top region, `+` spawns a shell tab on the shared stack from Phase 2. | med-high | The only genuinely new backend surface in this spec; needs its own smoke test (resize, PTY exit, tab close mid-command). |
| **6** | **Keyboard nav + polish (both pane types).** `↓`/`↑` fork cycling (agent, gated per §6.5 of the June spec), tab-close confirmation for `attention` tabs, drag-reorder if desired, persistence smoke tests. | low | UX polish on a working base; independently deferrable. |

Phases 1–2 are shared infrastructure and should land first regardless of which pane type ships
next. Phases 3–4 (agent) and Phase 5 (terminal) can proceed in parallel once Phase 2 lands,
since they touch disjoint view trees.

---

## 7. Non-goals (v1)

- Multi-session-per-block runtime for either pane type (rejected for agent panes in the June
  spec §8; the same reasoning applies to terminal panes — one PTY per block stays true).
- Tearing a tab off into its own top-level pane/tab (later; reuse the existing tear-off saga
  the June spec already flags as out of scope for forks).
- Terminal tab persistence across a full app restart beyond what block/layout persistence
  already provides for any pane (i.e. no special "restore exact shell state" beyond cwd).
- Renaming/reordering tabs via drag — nice-to-have, deferred to Phase 6 at earliest.
- Cross-tab features (split view, side-by-side compare) — out of scope; this is strictly "one
  active tab, others dormant," matching the editor's own model.

## 8. Files this would touch (orientation, not a change-list)

- **Shared strip:** new `frontend/app/view/shared/PaneTabStrip.tsx`,
  `frontend/app/view/shared/_pane-tab-strip.scss`; refactor
  `frontend/app/view/editor/editor-tab-strip.tsx` and the relevant rules in
  `frontend/app/view/editor/editor-view.scss` to consume it.
- **Layout store:** wherever `LayoutNode`/tile state lives today
  (`agentmux-common/src/layout_types.rs` backend type; the frontend layout/tile store consuming
  it) for `blockStack`/`activeBlockId`.
- **Agent:** `frontend/app/view/agent/agent-view.tsx` (wire the new top region), reuse
  `frontend/app/view/agent/fork/fork-set.ts` (`computeForkSet`) unchanged; retire
  `ForkBar.tsx`'s `PaneRow`-based rendering in favor of `<PaneTabStrip>` (keep `PaneRow` itself
  — the `ActivityDock` still uses it for pinned processes, that usage is untouched).
- **Terminal:** `frontend/app/view/term/term.tsx`, `frontend/app/view/term/termViewModel.ts`
  (new top region + tab-switch wiring); new RPC handler near
  `agentmux-srv/src/server/app_api/pane.rs` (`open_pane_floating`'s sibling); no changes to
  `agentmux-srv/src/backend/blockcontroller/shell/**` (PTY/controller machinery reused as-is).
- **Explicitly not touched:** `agentmux-srv/src/backend/shell_node.rs`/`ShellSessionRegistry`
  (confirmed wrong fit, §2.3) — no changes there.

## 9. Open questions

1. **Top-placement deviation (§3.3)** — sign-off needed: does moving the fork bar from the
   June spec's bottom region to the top (to match the editor) still preserve the "stay near
   the composer" ergonomic the June spec wanted, or was that ergonomic more important than
   visual consistency? Recommendation in this doc: top, for the reasons in §3.3.
2. **Dormant-tab lifecycle for terminal panes** — does a background terminal tab keep its shell
   process alive (keep-alive) or get suspended, mirroring the agent side's open §6.6 question?
   A shell has no natural "suspend and resume with history" analog to `--resume` — a suspended
   terminal tab most likely just means "PTY killed, tab shows a re-launch affordance," which is
   a materially different UX from the agent side's seamless resume. Needs its own decision.
3. **`newpanetab` RPC naming and exact request/response shape** — placeholder name in §4.2;
   should follow whatever naming convention `pane.open`/`open_pane_floating`'s siblings use.
4. **Should `PaneRow` (the agent pane's pinned-process chrome) and the new `PaneTabStrip`
   share any code**, given both are "a row of status-accented things"? This spec keeps them
   separate (different shape: `PaneRow` is a vertical list of expandable rows for
   *processes*, `PaneTabStrip` is a horizontal strip of *mutually-exclusive views*) — worth
   a second look once both exist, not a blocker to shipping either.
5. **Overflow behavior** when many tabs don't fit the strip width — the editor's strip
   currently just compresses tabs to a min-width with no overflow chip (Phase 2 TODO noted in
   `editor-tab-strip.tsx`'s own header comment). This spec inherits that limitation for all
   three pane types; a real overflow menu is a shared follow-up, not specific to this feature.
