# Spec: the person sets their own Kanban stages (number, names, colour) in Settings, opened from a cog on the board

**Status:** active. Approved by the operator (relayed by AgentY, 2026-11-18); decisions D1 to D7 stand as recommended. Kanban tasks #47816 (this spec) and #47817 (its build); build PRs #7729 (data and API), the editor PR follows.
**Date:** 2026-11-16
**Author:** Naki-2@narko (Kanban's owner; lead AgentX). Filed by Agent2 and moved to AgentY, who gave it to me.
**Verified against:** `agentmux` `main` @ `6de8ace3fb` (2026-11-16), by reading code. Nothing here has been run.
**Changes:** `SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md` §4 and §7 (the eight columns and their colours become the default stage set), `SPEC_SHARED_KANBAN_ACROSS_MACHINES_2026_11_01.md` §9 (a board gains a stage set).

## 1. Ask

The operator, 2026-11-16, typed in Agent2's pane (quoted):

> "we need to be able to set our own kanban stages. lets get the latest compact UI for doing that. theyd be able to set the numer and names, and anything else improtant, like color. add edge cases too. add a cog icon to the top right, and that opens the kanban settings in the settings pane, which is where you should build it. we also need to think about the network tiers and themes"

So:
1. The person chooses how many stages the board has, what each is called, and its colour.
2. The editor is compact, in today's UI kit, in the Settings pane as a Kanban section.
3. A cog at the top right of the Kanban pane opens that section.
4. Edge cases, the network tiers (this machine, LAN, cloud) and themes are part of the spec.

## 2. Summary

- **A stage is a named, coloured place on the board that shows some of the board's eight built-in "sources"** (Backlog, In Research, Spec'd, In progress, Needs you, Review, Merged, Released: today's columns). A task still lands where its state, kind, PR and holder put it (`columnOf`); the person decides which stage shows each source. Nothing about a task changes, so the Tasks tool, the queue's states and old builds are untouched (§3).
- **The person can:** rename a stage, recolour it, reorder, merge sources into one stage ("Merged" and "Released" become "Done"), split a source by task kind (§4.3), add and remove stages, and reset to the default eight. From 2 to 12 stages (§5).
- **Needs you stays.** It is the one stage that asks the operator for something. It can be renamed, recoloured and moved, never removed or merged (§5, D4).
- **A stage set belongs to a board** (a new `stages` column on `db_boards`), so a shared board looks the same on every machine and a client board can differ from the main one. "Use for all my boards" copies one set to the rest (§6).
- **Agents can set stages too** through `Tasks board_edit` (`stages`), like every other board action. srv validates; it never repairs silently (§7).
- **Colours are theme tokens.** The person picks from a palette of named tokens; each is an alias of a colour every theme already tunes, and a test keeps them apart and readable in every theme, light and dark, high-contrast included (§9).
- **The editor works on a draft** with Apply and Discard, because the rules span rows (§8).
- **Phases:** B1 renames, recolours, reorders, merges, adds, removes and resets (2 to 8 stages), with the cog, the Settings section and the MuxAPI; B2 splits by kind (up to 12); B3 replication on LAN and cloud once the sync layer exists (§12).

## 3. Today (verified in code)

- **Eight columns, computed.** `COLUMNS` in `frontend/app/view/kanban/kanban-columns.ts` lists `backlog, research, specd, progress, needs, review, merged, released`, with fixed English labels. `columnOf(task)` computes the column from the stored state, the task's kind, what it starts after, its PR and its holder's live state. No column is stored (the stages spec §4.3).
- **Queue states (srv):** `open, claimed, done, failed, cancelled, held, blocked, review` (`crates/srv-core/src/backend/storage/work_queue.rs`, `work_state`). `columnOf` maps them:

| State and condition | Source column |
|---|---|
| `open` or `held`, and its spec was accepted (`isSpecd`) | `specd` |
| `open` or `held`, otherwise | `backlog` |
| `claimed`, a research or spec kind, holder fine | `research` |
| `claimed`, any other kind, holder fine | `progress` |
| `claimed` with a PR link, PR not merged | `review` |
| `claimed`, PR merged | `merged` |
| `claimed`, holder gone, waiting, limited, signed out, errored or stopped | `needs` |
| `blocked` (asked a question), `failed` | `needs` |
| `review` (handed in) | `review` |
| `done`, PR released | `released` |
| `done`, otherwise; `cancelled` | `merged` (cancelled in a folded group) |
| an agent working with no task ("untracked") | `progress` (a dashed card) |

- **Colours.** Eight tokens, `--kanban-stage-backlog … --kanban-stage-released`, set on `:root` in `frontend/app/theme.scss` as aliases of tokens each theme already tunes; seven themes override one or two. `kanban-stage-colours.test.ts` checks the eight stay apart and keep 3:1 against `--main-bg-color` in every theme.
- **Where state about columns lives.** A pane's `kanban:collapsed` meta holds column ids; `kanban:reviewlimit` and `kanban:closeonmerge` are settings (`schema/settings.json`) listed today in the Advanced section's index; `kanban:reminders` is a setting with a switch in the Kanban toolbar.
- **Boards.** `db_boards` (`crates/srv-core/src/backend/storage/work_boards.rs`) holds `id, slug, name, tier, readable, agents, is_default, archived, next_num, aliases, pending_delete_at, created_*`. `tier` is `local` today; `lan` and `cloud` arrive with the sync layer (not built). No column for stages. `board_json` in `crates/srv/src/server/work_boards.rs` is what `GET /agentmux/work/boards` returns; `POST` takes `action: create | edit | archive | unarchive | delete | cancel_delete | default`. An agent may do each (the board's history names it).
- **Settings.** `SETTINGS_SECTIONS` (`frontend/app/view/settings/settings-model.ts`) lists thirteen sections, labels from `t("settings.sections.*")`; the open section is the pane's `settings:section` meta. `openSettingsAt(section)` (`open-settings.ts`) opens or focuses Settings on one. There is no Kanban section.
- **The Kanban's toolbar** (`kanban-section-parts.tsx`, `KanbanToolbar`): board picker, find box, repository select, "Reports to", Lanes, Untracked and Reminders switches, **New task**, and a Refresh icon button at the right end. The picker's menu already has "Manage boards…" with the `gear` icon.
- **The Tasks tool names a column in two answers:** "now waits in the operator's Needs you column" (`crates/mcp/src/tools/work.rs`, and its test in `call_tests.rs`).

## 4. The model

### 4.1 Sources

The eight built-in column ids are the **sources**: `backlog, research, specd, progress, needs, review, merged, released`. They are fixed, computed by `columnOf`, and stable ids (they are what `kanban:collapsed` already stores). A future ninth source is possible; an older client ignores one it doesn't know (§10).

### 4.2 A stage

```
StageSet  { version: 1, stages: Stage[] }
Stage     { id: string,            // "backlog".."released" for the default set; "st-" + 6 base36 for a new one
            name: string,          // §5.2
            colour: ColourId,      // §9.1
            takes: SourceId[],     // the sources it shows, at least one
            kinds?: string[] }     // §4.3; absent or empty: every kind
```

The stage set of a board is stored as JSON text in `db_boards.stages` (empty means the default set; the default is never written, so a build can improve it). `GET /agentmux/work/boards` returns it as `stages` on each board (the effective set: the stored one, or the default).

The **default set** is today's board, one stage per source, with today's names and colours. A board with no stored set renders exactly as it does now.

### 4.3 Which stage a task is in

1. Compute its source with `columnOf` (unchanged).
2. Among the stages that take that source, the one whose `kinds` contains the task's kind (trimmed, case-insensitive, as `RESEARCH_KINDS` matches) wins.
3. Otherwise the one that takes the source with no `kinds` filter (the catch-all). Validation (§5.1) guarantees exactly one exists, so no task is ever unplaced.

Specificity decides, not order, so reordering stages never changes where a card lands. An untracked card has the source `progress` and no kind, so it lands in the catch-all for `progress`.

**Why only kind as a split.** `kind` is the one field the person already sets that cleanly divides work (spec, research, build, review, repro). More stages than sources only mean something if something can fill them; kind is cheap, computed, and the same in every client. Splitting by agent, repository or label can follow (§13).

### 4.4 What this deliberately is not: manual placement

A stage the person drags cards into by hand would need a stored stage on each card. That breaks "every column is computed, none is stored" (the stages spec §4.3): an older client would show the card in the wrong place, the home machine owns a task's lifecycle (shared-Kanban spec §3.3) so a hand-placed stage would fight a state change, and an agent finishing a task would leave the card where the person parked it. Stages show what the queue already knows. If the operator wants parking lanes later, `takes` can gain a `manual` source without changing this model (D3).

## 5. Rules

### 5.1 The set

| # | Rule | Why |
|---|---|---|
| R1 | 2 to 12 stages | The least is Needs you plus one that shows everything else; twelve fit the wide board's scroll and the narrow list |
| R2 | Every source is taken by exactly one catch-all stage (a stage with no `kinds` that takes it) | No task is ever unplaced |
| R3 | A source may also be taken by any number of kind-filtered stages; two stages may not take the same source with the same kind | Placement stays unambiguous |
| R4 | `needs` is taken by exactly one stage, which takes nothing else and has no `kinds` | The one stage that interrupts the operator stays whole; its count drives the badge and tab title (below) |
| R5 | A stage takes at least one source | An empty promise can never fill |
| R6 | Stage ids are unique | |

Without a kinds filter a set has at most eight stages (one per source); the kind split reaches twelve (B2).

**The Needs-you count is not a stage.** The Kanban tab title and the widget-bar badge count `needsCountOf` (tasks whose source is `needs`), whatever the stage layout is called.

### 5.2 Names

- Trimmed; 1 to 24 characters (grapheme clusters), no control characters. Any script, emoji and right-to-left text are allowed.
- Unique within the board, compared case-insensitively after trimming ("Done" and " done" collide).
- The words "Needs you" belong to the `needs` stage: any other stage may not take that name (a stage called "Needs you" that doesn't ask the operator for anything misleads).
- Whitespace-only and empty names are refused with the reason shown next to the field; the Apply button stays off.
- Names are the person's text: never translated, never changed by an update. The default set's names come from the app (they follow the language setting); once the person edits a name it is stored as typed.

### 5.3 Colours and the fewest stages

Two stages may share a colour: colour reinforces, the written name carries the meaning (WCAG 1.4.1), and a person with one stage per source and ten stages has more stages than distinct colours. The editor does not warn.

## 6. Where it is stored, and who sees it

**On the board**, in `db_boards.stages` (JSON, a `version` inside it). Not in `settings.json` and not in pane meta:
- A board is the unit that is shared across machines (shared-Kanban spec §9). Settings are this machine's file (the reminders switch is per machine, and I found no sync of `settings.json`), so a set kept there would differ across the machines of one shared board.
- A person with a "work" board and a "client" board can keep different layouts, and an archived or deleted board takes its stages with it.
- Pane meta is per pane; two panes would disagree.

**Applying one set to every board.** "Use these stages for all my boards" in the editor copies the draft to every live board the person can edit, after a confirmation that says how many boards. It is one action per board on the API; nothing links the boards afterwards.

**The default for a new board** is the built-in default set. (Copying from another board at creation is a follow-up, §13.)

**A board that is archived** is read-only: its stages can't be edited, its stage set still renders. A deleted board takes its stages with it (the 15-second veto of §9.2 of the shared spec applies as before).

## 7. The API

All through the existing board route, so a container agent has it the same as a native one (the shared spec §9.3).

- `GET /agentmux/work/boards` and `Tasks boards`: each board gains `stages` (the effective set, the `StageSet` of §4.2) and `stages_rev` (a counter that goes up on each change, for §8's conflict check).
- `POST /agentmux/work/boards` `action: "edit"` gains `stages` (the whole new set) and `stages_reset: true` (clear to the default). `Tasks board_edit` gains the same two parameters. The request may carry `stages_rev`; when it is present and doesn't match, srv refuses with "the stages changed since you read them" and returns the current set.
- **srv validates every rule of §5 and refuses with a list of reasons; it never repairs.** (A silent repair would make an agent's call do something other than what it asked.)
- **Who may:** the same as renaming the board: the operator, and an agent that sees the board (a board's agent list can keep an agent out). The board's history gets a line naming the caller: "changed the stages: 8 → 6" (an agent's name for an agent, never the operator's).
- **The Tasks tool does not name stages.** A tool answer says what happened in state words ("now waits for the operator"), never a column the person may have renamed. The two answers that say "Needs you column" are reworded in the build. Nothing else about `Tasks` changes: `list`, `claim`, `move` and the states are as today. (Adding a `stage` word to a task row would need `columnOf` in Rust; it is a follow-up, D7.)

## 8. The UI

### 8.1 The cog

An `IconButton` (`gear`, label "Kanban settings") at the right end of the Kanban toolbar, after Refresh: the top right of the pane. It calls `openSettingsAt("kanban")` with the pane's selected board, so the editor opens on the board the person was looking at. A command-palette entry "Kanban settings" does the same. `openSettingsAt` gains an optional second argument that sets `settings:kanban-board` in the Settings pane's meta; when Settings is already open, it is pointed at the section and the board.

### 8.2 Settings, a Kanban section

A new section `kanban` (rail icon `table-columns`, label `t("settings.sections.kanban")`), placed after Agents and before Advanced. It also takes the two Kanban rows now listed in Advanced (`kanban:closeonmerge`, `kanban:reviewlimit`); the search index moves them with it. The review limit's words change from "Review" to "the stage that shows Review".

The editor is compact (UI kit `density="compact"`, as the Kanban toolbar):

```
Kanban                                              [Board: Main ▾]  (this machine only)
─────────────────────────────────────────────────────────────────────────────────────────
 Stages   8                                                    [ Reset ]  [ + Add stage ]
 ┌──────────────────────────────────────────────────────────────────────────────────┐
 │ ⋮⋮  ● Needs you   [Shows: Needs you]                         · required     ⋯      │
 │ ⋮⋮  ● Backlog     [Shows: Backlog, Spec'd ▾]                                  ⋯  ✕ │
 │ ⋮⋮  ● Doing       [Shows: In Research, In progress ▾]   [Kinds: any ▾]       ⋯  ✕ │
 │ ⋮⋮  ● Review      [Shows: Review ▾]                                           ⋯  ✕ │
 │ ⋮⋮  ● Done        [Shows: Merged, Released ▾]                                 ⋯  ✕ │
 └──────────────────────────────────────────────────────────────────────────────────┘
 Preview   ▌Needs you 2  ▌Backlog 9  ▌Doing 4  ▌Review 3  ▌Done 31
 [ Use for all my boards ]                                   [ Discard ]  [ Apply ]
 ─────────────────────────────────────────────────────────────────────────────────────
 Review limit  [ 0 ]    Close a task when its PR merges  [on]
```

- **Rows:** a drag handle, the colour (a button showing the swatch and its name; its popover lists the palette, each swatch with its name written), the name (a text input with a visible label for screen readers), "Shows" (a multi-select of the sources not claimed as a catch-all elsewhere; choosing a source another stage holds as its catch-all moves it, with the move named in the row's note), "Kinds" (B2), a `⋯` menu and a remove button.
- **Needs you** has no remove button, its Shows is fixed, and the row says "required" (R4).
- **Keyboard:** Up and Down move a focused row with Alt held (and the `⋯` menu has Move up and Move down); Add moves focus to the new row's name; Remove moves focus to the next row; Esc in a popover returns to its button.
- **Validation:** each problem is written beside the field it concerns and listed at the foot as one live-region message; Apply is off while any rule fails.
- **Preview** is a strip of the stage headers with today's counts for this board, in the real colours and order: the way to see a merge before applying.
- **Draft, Apply, Discard.** The editor edits a copy. Apply sends it with the `stages_rev` it started from; a mismatch (another machine, or an agent, changed the stages meanwhile) shows "These stages changed while you were editing, by <name>: [Load theirs] [Apply mine]". Leaving the section with unsaved changes asks once.
- **The board select** lists the boards the person can edit, with their tier icons. An archived board is listed disabled.

### 8.3 The board

The board renders by stages instead of `COLUMNS`:
- A stage header keeps today's look: a 3 px bar and a dot in the stage colour, the name, the count, and the stats strip when collapsed.
- **Which stages fold, how they sort:** in the narrow view (760 px or under), Needs you comes first, then the stage order. A stage folds by default when every source it takes is a finished one (`merged`, `released`).
- **The collapsed set** (`kanban:collapsed`) holds stage ids. The default set's ids are today's column ids, so an existing pane keeps its collapsed columns. A board switch reads the set through the new board's ids and doesn't rewrite the meta, so switching back finds it.
- **Agent groups and drag and drop.** Today's Backlog and Spec'd groups group waiting cards by agent and are drop targets (`takesDrops`). The rule becomes: a stage whose sources are all waiting ones (`backlog`, `specd`) groups by agent and takes drops. A stage that mixes waiting and other sources is a flat list with no drop target; the agent strip, and the card menu's "Move to…" and "Start on…", still work.
- **Cards** are unchanged: the agent stripe, the word pills ("spec", "build") on cards whose source is `needs` or `review`, the on-hold pill. The Needs-you card border follows the source, not the stage's colour.
- **The Review limit** counts the tasks whose source is `review` in the stage that shows it, and says "over N" on that stage's header.
- **Lanes mode** repeats the same stages in every repository's row.
- **The untracked cards** are in the stage that shows `progress`.
- **The Cancelled group** (folded) sits under the stage that shows `merged`.

## 9. Themes

### 9.1 The palette

A colour is a **palette id**, never a hex. The palette is the eight existing colours, under neutral names, plus up to two more:

| Id | Token (alias set on `:root`) | Today's meaning |
|---|---|---|
| `muted` | `--kanban-stage-backlog` → `--secondary-text-color` | Backlog |
| `research` | `--kanban-stage-research` → `--process-kind-agentmux` | In Research |
| `info` | `--kanban-stage-specd` → `--info-color` | Spec'd |
| `active` | `--kanban-stage-progress` → `--process-kind-agents` | In progress |
| `attention` | `--kanban-stage-needs` → `--attention-color` | Needs you |
| `review` | `--kanban-stage-review` → `--process-kind-builds` | Review |
| `done` | `--kanban-stage-merged` → `--process-kind-shells` | Merged |
| `shipped` | `--kanban-stage-released` → `--process-kind-runtimes` | Released |
| two more | `--kanban-stage-extra-1`, `-2` | the build picks the aliases that pass the test below; if none pass in every theme, the palette stays at eight |

The default set uses the same colours as today. The existing token names stay; the ids are what is stored. The ids are the role names the editor shows (Muted, Research, Info, Active, Attention, Review, Done, Shipped). The first build stored the hue names (`grey`, `cyan`, `violet`, `magenta`, `amber`, `yellow`, `green`, `blue`): a stored or sent set that uses them is still read and accepted, and is written back with the role name.

### 9.2 How it reaches the screen

- TSX sets a **class or data attribute** from the id (`data-stage-colour="done"`), and `kanban.scss` maps each to `var(--kanban-stage-…)`. No inline colour, no `#hex`, `rgb()` or `hsl()` in TSX or SCSS, no `var()` fallbacks on a token that is always set (the UI-primitives ratchet counts both; its per-file counts must not rise), and no palette utilities in Tailwind classes (`npm run lint:theme-colors`).
- Colour is on the bar and the dot only; column bodies and cards stay uncoloured (the board's "borders, not fills" rule).

### 9.3 What the test checks

`kanban-stage-colours.test.ts` is extended from eight to the whole palette. In **every** theme, light and dark (a light theme with the light-polarity block applied), including **high-contrast**:
- each palette colour keeps at least 3:1 against `--main-bg-color` (WCAG 1.4.11);
- the palette stays apart by a minimum OKLCH distance.

A theme that needs it overrides an alias in its own file, as seven do now. The editor shows each swatch **with its name written** (the swatch is not the only signal), and the board always writes the stage's name. Under the operating system's forced-colors mode the bar and dot take the text colour and the name carries the stage; the build checks it in a forced-colors browser pane.

## 10. Edge cases

| Case | What happens |
|---|---|
| **Remove a stage that holds tasks** | Nothing happens to the tasks: their state, kind and PR still put them in the same source. Removing a stage whose sources would be left with no catch-all asks where they should show: "Remove 'Review' and show its sources in: [Doing ▾]" (default: the stage before it, the next for the first). A kind-filtered stage just returns its tasks to its source's catch-all. The row says how many tasks are in the stage now. Discard undoes it; Apply is the commit |
| **The queue's states** (waiting, claimed, review, done, failed, held, blocked, cancelled; "needs you") | Every state maps to a source (the table in §3) and R2 guarantees a catch-all for each source, so each state shows somewhere, whatever the person did to the stages. A held task is still its source's stage, with the "on hold" pill. A failed or blocked task is always in the `needs` stage |
| **Renaming against the state names agents use** | Agents and the Tasks tool use states (`open`, `claimed`, …) and `kind`; a stage's name is display only and nothing in the API depends on it (§7). A stage called "Done" doesn't mean the state `done` |
| **Fewest** | Two: Needs you and one stage that takes the other seven sources. The editor says so at the limit (Remove is off, the reason in its tooltip) |
| **Most** | Twelve; Add is off at the limit with the reason beside it. Without kind filters the practical most is eight (R2, §5.1) |
| **A new stage with nothing to show** | It must take a source (R5): "Add stage" opens its row with Shows open, and Apply stays off until it has one |
| **Duplicate, empty, long, odd names** | §5.2: refused beside the field. " Done" and "done" collide; a 25th character is refused (the input stops there, with the count shown); a name that is only emoji is fine; right-to-left text keeps its direction in the row and on the board; a name that matches the reserved "Needs you" is refused on any other stage |
| **Several boards** | A set per board (§6). "Use for all my boards" copies to every live board the person can edit; a board that can't be edited (archived, or an agent list that excludes the editor) is listed as skipped in the result, not silently ignored |
| **Two editors at once** | The `stages_rev` check of §7 and §8.2: the second Apply is refused with the other change shown, and the person chooses |
| **An agent changes the stages while the editor is open** | Same check; the editor shows "Changed by <agent>" and offers Load theirs. The board itself updates at once |
| **A board read from another host** (`Tasks list` with `host`, or the pane showing a peer's board) | The peer sends the board with its stages; the reader draws it with that set, because the board is the unit (§6). The reader cannot edit it unless the board's rules allow it (below) |
| **A peer with no stages (an older build)** | The answer has no `stages`, so the board draws with the default set. Nothing breaks and nothing is written back |
| **An older Kanban client on a newer srv** | It ignores `stages` and shows the eight columns. Tasks are unaffected |
| **A newer client on an older srv** | The board has no `stages`; the editor is shown with "This machine's AgentMux can't keep custom stages yet" and its Apply off |
| **A stored set the client can't use** (hand-edited, or a `version` this client doesn't know, or one that breaks R2) | The board draws with the default set and the editor shows a banner with the reason and the board's name, and Reset. The stored text is not overwritten until the person acts |
| **An unknown colour id or source id** (a future palette or source) | An unknown colour draws as `grey`; an unknown source in `takes` is ignored; a stage left with no known source is hidden and the editor says so |
| **A stage that shows `released` for a private repository** | Private repositories' tasks reach Merged and never Released (the stages spec §4.5); that stage shows empty. The editor's tooltip on `released` says so; it isn't an error |
| **A kind that no task has, or a task whose kind changes** | A kind filter is a string match computed on each render; a kind nobody uses leaves the stage empty, and a task whose kind is edited moves stages at once. Kinds are trimmed and case-insensitive |
| **Untracked cards** | In the stage that shows `progress`, with no kind, so in its catch-all |
| **The Needs-you badge and tab title** | The count of source `needs`, independent of the stage layout (§5.1) |
| **A stage collapsed in a pane, then the stage is removed or the board switched** | The collapsed ids are filtered through the board's set on read and never rewritten by a board switch (§8.3) |
| **The narrow view** | Needs you first, then the stage order; finished-only stages fold by default; every stage is a sticky section header in its colour |
| **Archived board** | Its stages show and can't be edited; the board select lists it disabled |
| **Move a task to another board** | No effect on stages: the task lands by its source in the other board's set |
| **Delete a board** | Its stages go with it (the 15-second veto as for any board delete) |
| **The Review limit** | Counts the `review`-source tasks in the stage that shows it. If Review is merged into "Doing", the limit applies to the count of review-source tasks in Doing, said on its header |
| **Reminders, claim, hold, accept** | They act on states and kinds, never on stages: unchanged |

## 11. Network tiers

| Tier | A stage set means | Who may change it | What another party sees |
|---|---|---|---|
| **This machine only** (`local`, the only tier built today) | The `stages` column of the board in this machine's database | The operator in Settings, and any agent on this machine that sees the board (Tasks `board_edit`) | Nothing leaves the machine |
| **My machines (LAN)** (when the sync layer exists) | A board-level field of the shared board, replicated like the board's name | Any member machine's operator, and any agent that sees the board; the lifecycle rule (home machine only) concerns tasks and doesn't apply here | Every member machine draws the board with the same set. A member on an older build draws the default eight |
| **My machines and AgentMux Cloud** | The same field; for a board the cloud may read (`readable`), the stage names and colours are as readable as the board's name; for one it may not, they are inside the board's private part, as the name is | The same | The same, plus the cloud's console shows the stage names only on a readable board |

**Merge.** The stage set is one value with a counter. Two members editing it at once give "newest wins" for the whole set (the shared spec's rule for content fields), and the overwritten set stays in the board's history with a **Restore** action in the editor. The online check of §7 makes a blind overwrite unlikely; offline edits that lose are shown in the board's history, not discarded.

**What stays per machine:** the collapsed stages (a pane's meta), the toolbar's switches, and which board a pane selected.

**A board read from another host** is drawn with the stages that host sent for it (§10). Writing needs the board's rules to allow it; a read-only view is clearly labelled in the editor ("Another machine's board: stages can't be changed from here").

**Dependency.** The LAN and cloud rows wait for the sync layer (K1). B1 and B2 are complete on their own for a local board and do not depend on it.

## 12. Tests and phases

### 12.1 Tests

- **Rust (srv-core):** a pure validator for §5 (each rule, with the reason strings); `db_boards.stages` round-trip, the empty means default, and an unknown `version` is stored as given and read as default with the reason; `stages_rev` increments and a stale one is refused; `board_edit` with `stages` and `stages_reset`; an agent not in a board's agent list is refused; the history line names the caller; the Tasks tool answer no longer says "Needs you column". Heavy srv tests run on charlie.
- **Frontend (vitest):** `stageOf(task, set)` for every state in the §3 table against the default set, a merged set, and a kind-filtered set (specificity, order-independence, untracked); R1 to R6 in the editor model; Add, Remove-with-reassign, merge, reorder by keyboard; the draft, Apply, Discard, the conflict banner; the cog calls `openSettingsAt("kanban", board)`; the Settings section registers and its rows are in the search index; the collapsed-ids read filter.
- **Behaviour snapshots** (the pattern `kanban-section.behavior.test.tsx` uses) for the toolbar with the cog, the editor at the default, at a merged set, at the minimum and the maximum, with an error shown.
- **Themes:** `kanban-stage-colours.test.ts` over the whole palette in every theme (§9.3). The UI-primitives ratchet, `lint:theme-colors`, the function-size gate and the i18n literals ratchet pass.
- **Look checks:** macOS and Linux, per the workspace rule, on the editor (compact density, the swatch popover, drag), the board at wide and narrow widths with a merged and a split set, in a dark and a light theme and high-contrast, and at 100 % and 150 % zoom.

### 12.2 Phases

| Phase | What | Notes |
|---|---|---|
| **B1** | `db_boards.stages` and its validator; the API (`stages`, `stages_rev`, `board_edit`); the board renders by stages; the Settings section and editor (rename, recolour, reorder, merge, add, remove, reset, preview, draft and conflict check, "use for all"); the cog and command-palette entry; the palette and its test; the two Tasks answer rewordings; the Review limit and `kanban:closeonmerge` move into the section | 2 to 8 stages. One PR, or two if it is large: the data and API first, then the editor |
| **B2** | `kinds` on a stage: the filter, R3, the editor's Kinds control, the placement rule of §4.3 | 9 to 12 stages. Only if D2 is yes |
| **B3** | Replication of the field on LAN and cloud, the history line and Restore across machines, the read-only label on another host's board | Waits for the sync layer (K1) |

## 13. Out of scope, follow-ups

- Manual placement and parking lanes (§4.4).
- Splitting by agent, repository or a label; a stage "shown only to" an agent or a person.
- A `stage` word on `Tasks list` and `get` rows (needs `columnOf` in Rust).
- Copying stages from another board when a board is created.
- Per-stage limits other than Review's (a work-in-progress limit for any stage).
- Importing and exporting a stage set as a file.
- Swarm's task chip showing the stage's colour (the stages spec §9 plans a stage dot there; it would read the board's set).

## 14. For the operator

Recommendations (D1 to D7); each stands unless you say otherwise:

- **D1: stages belong to a board**, with "Use for all my boards" to copy. The alternative, one set for everything on this machine, breaks a shared board (machines would disagree) and a client board that wants its own layout. *Recommend board.*
- **D2: more than eight stages need a split by kind.** Without it the most is eight, the number of things the queue can tell apart. *Recommend yes, as B2*, after B1 is in your hands.
- **D3: no card placed by hand.** Stages show where the queue already puts a card; a parking lane is a separate feature (§4.4). *Recommend no.*
- **D4: Needs you stays, whole.** It can be renamed, recoloured and moved, not removed or merged: it is the one stage that asks something of you. *Recommend yes.*
- **D5: agents may set stages** through `Tasks board_edit`, recorded under the agent's name, like every board action. *Recommend yes* (the workspace rule: agents can do anything but forge your voice).
- **D6: the editor works on a draft** with Apply and Discard, since the rules span rows, rather than saving each keystroke. *Recommend draft.*
- **D7: no `stage` word in the Tasks tool yet.** Agents keep working in states. *Recommend later.*

## 15. Files

**Frontend** (`frontend/app/view/kanban/`): `kanban-columns.ts` (`COLUMNS` becomes the default set; `stageOf`; collapsed ids), `kanban-board.tsx` and `kanban.scss` (render by stage; the colour attribute), `kanban-section-parts.tsx` (the cog), `kanban-section-state.ts`, `kanban-boards.ts`, `kanban-types.ts` (`stages`, `stages_rev` on `KanbanBoardInfo`), new `kanban-stages.ts` (the model and the validator mirror), `kanban-stage-colours.test.ts`.
**Settings** (`frontend/app/view/settings/`): `settings-model.ts` (the `kanban` section), `open-settings.ts` (the board argument), a new `sections/kanban-section.tsx`, `kanban-section-parts.tsx`, `kanban-section-state.ts`, `kanban-settings-index.ts`; `frontend/app/i18n` strings.
**Theme:** `frontend/app/theme.scss` and `themes/*.scss` (palette aliases and any overrides).
**srv** (`crates/srv-core/src/backend/storage/work_boards.rs`, `crates/srv/src/server/work_boards.rs`, `crates/srv/src/server/app_api/kanban.rs`): the column, the validator, the route fields, the history line. **mcp** (`crates/mcp/src/tool_schemas.rs`, `tools/tasks_tool.rs`, `tools/work.rs`): `stages` and `stages_reset` on `board_edit`, the two reworded answers. `frontend/types/srv-types.d.ts` (generated stubs).
