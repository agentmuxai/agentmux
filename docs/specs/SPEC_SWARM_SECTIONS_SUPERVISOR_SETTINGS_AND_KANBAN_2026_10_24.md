# Spec: the Swarm as three sections (Supervisor, Broadcast, Kanban); the Supervisor switched off until the operator turns it on, with its settings in the Supervisor section

**Status:** active. Phase A1 is built in PR #5560 (the master switch, the one-time reset, the limits as settings, the `set-mode` refusal, the attribution log; this PR); phase A2 (wait before continuing, the fleet budget, the notify toggles) is built in PR #5610, with the restart clean-up ("not continued: AgentMux restarted") in the PR after it; phases B to D are not built. The §9 decisions are built as recommended (AgentX assigned the build on 2026-10-26); the operator-only guards in §6.2 were dropped, see there.
**Date:** 2026-10-24
**Author:** Camper@narko
**Task:** Spec #2415; the builds are #2416 (the sections, the Supervisor settings: Camper) and #2417 (the Kanban in the Swarm: Naki-2), both after this is approved
**Requested by:** the operator, through AgentY. Their words: "the supervisor was actually a bit too active earlier, we'd rather have the supervisor off at first, but implement the UX and let the operator set the settings. Currently I DO NOT see the additional sections inside of the Swarm: Supervisor, Broadcast, and we now want to also move Kanban from Warden to Swarm."
**Verified against:** `agentmux` `main` @ `d5e74791d` (v0.60.0, 2026-10-24), by reading the code named below; re-checked against `55950215a` (2026-10-25), where the Swarm, Warden, Kanban, Supervisor and sections code is unchanged except Kanban card name colours. Nothing here has been run.
**Builds on:** [REPORT_SWARM_SECTIONS_AND_AMBIENT_IN_WARDEN_2026_10_21.md](../reports/REPORT_SWARM_SECTIONS_AND_AMBIENT_IN_WARDEN_2026_10_21.md) §4 (QTimer's design for the Swarm sections; the operator took every recommendation on 2026-10-21; its phase 1, the Warden Ambient section, is built), [SPEC_SWARM_SUPERVISOR_2026_10_16.md](SPEC_SWARM_SUPERVISOR_2026_10_16.md) (the Supervisor, built in #5036), [SPEC_PANE_SELECTION_STATE_MACHINE_AND_SWARM_CLICK_2026_10_18.md](SPEC_PANE_SELECTION_STATE_MACHINE_AND_SWARM_CLICK_2026_10_18.md) (why the agent list is never remounted), [SPEC_WARDEN_KANBAN_2026_10_20.md](SPEC_WARDEN_KANBAN_2026_10_20.md) and [SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md](SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md) (the Kanban that moves)

## 1. Ask

1. The Swarm becomes a pane of **sections**: **Supervisor**, **Broadcast** and **Kanban**. The first two are QTimer's design (the report's §4, decided 2026-10-21 and never built: QTimer was retired on 2026-10-23). The Kanban moves into the Swarm from Warden.
2. The **Supervisor is off until the operator turns it on**, and the operator **sets its settings in the Supervisor section**: which agents it watches, its limits, and which of its stops notify.
3. The report's decisions D1 to D7 stand unless the Kanban changes one (§9).

## 2. What is there today

- **The Swarm is one flat pane**, not a sectioned one. `swarmPaneTab` (`swarm.tsx:10`) declares no `sections`. `SwarmView` (`swarm-view.tsx`) is a single column: `AccountAvailabilityStrip`, `FleetToolbar` (select all, Broadcast, Stop, Supervisor, Clear), `FleetResultPanel`, an empty state or "Clear completed", `SwarmGroupByControl`, then the `.swarm-tree` list. The Stats panel the report describes is already gone (Warden's Ambient section replaced it). `swarm-view.tsx` is 1453 lines against a 1500-line limit.
- **Rows** are `AgentRowHost` over `AgentRow`, keyed by block id and selected on pointer-down (`revealBlock(blockId, { source: "swarm" })`). Selection is one `selectedBlockIdsAtom`. Broadcast is an inline composer that hides the other toolbar buttons while it is open.
- **The sections contract** (Naki-2, #5284 and after): `PaneSections` in `frontend/app/block/pane-sections.ts` (`default`, `items: [{ id, label, icon, sections? }]`), the selected id in block meta `<view>:section`, `sectionOf` (an unknown id falls back to the default), `sectionProblem`, a top `TabbedPane` (icons only when narrow). Pinning a section to the widget bar comes with it (#5288). Warden, Settings, Connectors and Memory declare sections.
- **The Supervisor in code** (`crates/srv/src/backend/supervisor/`): mode is **per pane**, in block meta `supervisor:mode` (`steady` or `drive`; absent is Off). **A new agent already starts Off**, so the code does not turn it on by default. It has **no settings at all**: no key in `schema/settings.json`, `settings-template.jsonc` or the wconfig types, and **no global switch**. Its limits are constants (`SupervisorMode::limits()`, `mod.rs`): Steady 5 continuations in a row and no time cap, Drive 15 and 2 hours, both stop after 2 continued turns that used no tool. The judge is Haiku 4.5 through the ambient gateway (a constant, 90 s timeout, one retry). Notifications go through the router's own global `notify:os:*` settings; there is no Supervisor-specific toggle. The only UI is a per-row chip and the toolbar's Supervisor menu.
- **Why it felt too active**, from the code: every completed turn on a supervised Claude pane is judged (up to two calls with the retry); Drive may send 15 continuations per user message over 2 hours, each a full agent turn on the operator's account; **nothing waits between a turn's end and its continuation**, and there is no cost or rate budget (spend is counted in `ambient/spend.rs`, not capped). The operator can only turn it off pane by pane.
- **The Kanban in Warden**: `warden-kanban/warden-kanban-manager.tsx` is the section root, a Warden section in `warden-sections.ts` (first in the rail), lazily mounted by `warden-view.tsx` with `visible`, `onNeedsCount`, `openRequest` + `onOpenHandled` (pane meta `kanban:open`) and `lanes` + `onLanes` (pane meta `kanban:lanes`). It decides narrow or wide by measuring `.warden-container` against 760 px, and its CSS is `@container warden`. A card, the drawer and the agent strip call the global `revealBlock(blockId, { source: "kanban" })`.

## 3. Design: the Swarm's sections and shared scaffolding

This spec (and #2416) owns the scaffolding; #2417 (Naki-2) builds the Kanban view that mounts in the slot below.

### 3.1 Shape

- `swarmPaneTab` declares `sections: { default: "supervisor", items: [supervisor, broadcast, kanban] }`, meta key `swarm:section`, rendered with the same top `TabbedPane` Warden uses. The title follows the section ("Swarm · Broadcast"; "Swarm · Kanban · 2 need you").
- **Rail order: Supervisor (`user-shield`), Broadcast (`tower-broadcast`), Kanban (`table-columns`).** This is the order the operator named in this request. (The report listed Broadcast first, from the earlier request; S1.)
- **Default: Supervisor** (the report's D1, unchanged with three sections): watching agents is what the Swarm is opened for. Broadcast and Kanban can be pinned to the widget bar, which the contract gives free (#5288).
- `defwidget@swarm` opens the default section. `defwidget@kanban` moves to `view: swarm, section: kanban` (§4).

### 3.2 One list, one selection (the report's §4.2, kept)

Supervisor and Broadcast show the same agents from the one `SwarmViewModel`: one poll, one set of event subscriptions, one `selectedBlockIdsAtom`, so a selection made in Broadcast is still there in Supervisor.

- **The list is rendered once.** It becomes a component, `SwarmAgentList` (`swarm-agent-list.tsx`), extracted from `swarm-view.tsx` with `AgentRowHost` and `AgentRow` **unchanged** in how they are keyed, how they take their node (an accessor), and how a press selects. Switching section changes only the action area above the list and a `density` the rows read (compact in Broadcast, full in Supervisor). A section switch never remounts a row, so #5208's fix holds: a click is lost when the pressed element is replaced before release.
- **The Kanban section has its own content** (its board, its agent strip), so it shows neither the list nor the selection. The list stays **mounted and hidden** (`display: none`) while Kanban is showing, so coming back keeps the same elements and the scroll position. A hidden list costs nothing to paint; its signals keep updating, as they do for any Swarm pane in a hidden tab.
- Hidden detail in compact mode (the child buckets) is collapsed, not unmounted, wherever that keeps the row's elements stable. The select tracing (`select-trace.ts`, the `traced()` setters) is untouched.
- **AgentY is told before the PR that extracts `SwarmAgentList`**, because it moves the code that holds `AgentRow` and `AgentRowHost` (the report's §9).

### 3.3 The sections

| | Supervisor | Broadcast | Kanban |
|---|---|---|---|
| Content | Full rows: status, Supervisor chip, line, held task, every child bucket | Compact rows: checkbox, icon, name, status chip, line | The board (#2417) |
| Action area | The Supervisor bar (§6.1) and its settings (§6.2) | The composer, always docked under the list | Its own toolbar |
| Stop N, Clear completed | Here (the report's D3) | | |
| Group-by control | Above the list | Above the list | n/a |
| Account availability strip | Above the sections, for all three | | |
| Selection | Shared | Shared | Not used |

- **Broadcast:** the multi-line composer, Send, the recipient count and the "N can't receive this" note (`gapSummary`), the per-target result (`FleetResultPanel`), and the last 10 broadcasts sent from this Swarm (text, when, recipients, delivered or failed), in memory only (the report's D5).
- **Remote agents** show as today; an action they can't take says why (`unavailableReason`). The Supervisor runs only for this AgentMux's agents.

### 3.4 The Kanban slot (the contract with #2417)

The Swarm mounts the Kanban lazily the first time its section is shown and keeps it mounted, as Warden does for its sections. The slot passes what `warden-view.tsx` passes today, plus one new hook:

```ts
interface SwarmKanbanSlotProps {
    visible: boolean;                       // the section is shown: pauses polling when false
    openRequest: string | null;             // a task id to open (the held-task chip asks for it)
    onOpenHandled(): void;
    onNeedsCount(n: number): void;          // feeds the title and the Needs-you badge
    lanes: boolean;                         // pane meta kanban:lanes (a boolean: lanes view on or off)
    onLanes(on: boolean): void;
    onOpenAgent?(blockId: string): void;    // select that agent's row in the Swarm; without it, revealBlock as today
}
```

- **Size.** The slot is a flex column child with a definite width and height (`.swarm-section-body`: `flex: 1; min-height: 0; min-width: 0`), because the board scrolls itself (`.kanban-board` has `overflow-y: auto` in the narrow view) and its sticky section labels and single scrollbar need a definite height. #2417 makes the slot a named container and measures its own width against `$kanban-narrow` (760 px) instead of `.warden-container`; the Swarm needs no knowledge of the name. The narrow single list, the extra-large labels (#2412) and the agent-colour names (#2263) move as they are.
- **Meta.** `kanban:open` (a task id) and `kanban:lanes` (a boolean) move from Warden's pane meta to the Swarm's (`srv-types.d.ts`, `sdk/widget-sdk/v1.d.ts`).
- **The held-task chip** (`HeldTaskChip`, in the Swarm rows) no longer opens another pane: `openKanbanTask` sets `swarm:section: kanban` and `kanban:open: <task>` on **this** Swarm block, so a click on a row's task flips the same pane to the Kanban with the task open.
- **A card's agent** (`revealBlock(..., { source: "kanban" })` in the card, drawer and agent strip) keeps working. With `onOpenAgent` the Swarm also switches to the Supervisor section and selects that row, so "go to this agent" lands on its row in the list.
- **Phasing.** The scaffolding lands with the Kanban item **absent from the rail** (a section is not offered until its view is there); #2417's PR adds the item and the lazy mount in one change.

## 4. The Kanban leaves Warden

What moves, what is removed, and what keeps working (everything found in the code):

| Where | Today | After |
|---|---|---|
| `warden-sections.ts` | `kanban` first in Warden's rail | Removed. Warden's default stays Host. |
| `warden-view.tsx`, `warden-model.ts` | The lazy block, `kanbanNeeds`, `kanban:open` / `kanban:lanes`, the title "Kanban · N need you" | Removed; the Swarm's model and title carry them |
| `open-warden.ts` (`openWarden("kanban")`, from `jobs-history.tsx`) | Opens Warden's section | Opens the Swarm's Kanban |
| `open:kanban` palette command | `warden:section: kanban` | Opens the Swarm's Kanban |
| `defwidget@kanban` (`widgets.json`) | `view: warden`, `section: kanban` | `view: swarm`, `section: kanban`; the Needs-you badge (`kanban-needs-badge.tsx`, `opensKanban`) tests the new pair. It rides the shared board fetch (#5308), so it needs no Swarm open. |
| A Warden pane saved on Kanban | `sectionOf` falls back to the default (Host), tested (`warden-view.test.tsx` "invalid section") | Unchanged: it opens on Host. No notice. |
| A user's own pin or widget naming `warden` / `kanban`; a widget SDK call `am.panes.open("warden", { "warden:section": "kanban" })` | Works | **Keeps working through a moved-sections table** in `pane-sections.ts`: `{ from: { view: "warden", section: "kanban" }, to: { view: "swarm", section: "kanban" } }`, read by the widget bridge, `sectionProblem` (an old pair is not flagged in Settings > Widgets), the pin resolution on the widget bar, `openWarden`, and srv's `resolve_section` for `widgets.json` |
| The Kanban manager's own `AccountAvailabilityStrip` | Rendered inside the manager | **Removed from the manager by #2417.** The Swarm owns the one strip above all three sections (§3.3); Warden no longer shows the Kanban, so nothing else needs it. Otherwise it would show twice. |
| Agent-facing text "Warden > Kanban": `operator-config-seed.json`, the Tasks tool description in `crates/mcp/src/tool_schemas.rs`, the doc comment in `app_api/kanban.rs` | Says Warden | Says Swarm > Kanban; the operator-config manifest version is bumped so agents get the new text. **That text and the manifest bump are AgentY's lane (operator config and MuxAPI text): #2417 tells AgentY before changing them, so the bump doesn't collide with theirs.** |
| Comments naming Warden's Kanban (`account-availability-strip.tsx`, `drag/drag-types.ts`) | Not user-facing | Updated, so a grep for "Warden" finds nothing stale |
| Docs | Warden docs page describes the Kanban | The Swarm page gets the Kanban and the other two sections; the Warden page drops it (`agentmux-docs`, in the same change set) |

**Warden does not keep a link.** A "Kanban moved to Swarm" stub in Warden would be a second place to maintain for a section whose entry points (command, widget, badge, chips) all follow the move (S3).

Tests that pin `kanban` today move with it: `section-pins.test.ts`, `widget-bridge.test.ts`, `pane-sections.test.ts`, `warden-view.test.tsx`, `kanban-needs-badge.test.tsx`, and the srv `layout_file` and `wconfig` tests.

## 5. The Supervisor is off until the operator turns it on

The code already starts every agent Off. What was missing is a switch above the agents, and a way back to a known state:

- **A master switch, `supervisor:enabled`, default `false`.** While false, the watcher judges nothing and sends nothing, and `is_supervised` is false, so the notification router sends its ordinary "finished" notices (`watcher.rs` `evaluate`, `mod.rs` `is_supervised`). `supervisor.set-mode` refuses Steady and Drive with "The Supervisor is switched off" (Off is always accepted). Nothing about an agent's stored mode is read while the switch is off.
- **One reset, recommended (S2).** At the first start after this ships, srv clears `supervisor:mode` on every block that has Steady or Drive, once (a startup migration, recorded so it runs once), through the same path as `set_mode(Off)`, which keeps each pane's decision history. It is recorded **once per home and per channel**, and is a no-op on a pane already Off. The changeset says plainly that existing Steady and Drive choices are cleared, because the operator will see their chips go to Off. Without it, turning the switch on would silently put back every agent the operator once set to Drive, which is what they found too active. With the switch off the reset changes nothing visible; it makes the first "on" a clean start.
- **A pane's own mode is kept** per pane, as today. The switch is the operator's one place to stop all of it.

## 6. The Supervisor section

### 6.1 The action bar (moves from the toolbar)

Set the mode for the selection (Off / Steady / Drive), **Stop N** (with its confirm and staged rollout), Clear completed. While the master switch is off the mode items are disabled with the reason ("Switched off. Turn the Supervisor on in its settings") and the per-row chip shows Off with the same tooltip. Remote agents say why they can't be supervised.

A small fix goes in with it: the row chip is offered for any Claude agent, but srv also needs `claude-stream-json` output (`supports`, `mod.rs`); a Claude pane with another output format shows a chip that then refuses. The chip follows srv's rule.

### 6.2 The settings

At the top of the section, one line when closed ("Supervisor: off") and a panel when open ("Settings"), so the list is not pushed down. The master switch is on the line itself. Every control writes a setting through the same setter the Settings sections use (`set` and `SettingRow` in `settings-controls`), takes effect on the next decision with no restart, and shows its default. The keys also appear in `settings.json`, in the Settings pane's search (one entry that opens this section), and in the schema, which means the four places a setting lives (`schema/settings.json`, `settings-template.jsonc`, the wconfig types, `srv-types.d.ts`).

| Setting | Key | Default | In code today? |
|---|---|---|---|
| **Supervisor on** | `supervisor:enabled` | **off** | New (§5) |
| Steady: continuations in a row | `supervisor:steady:continuations` | 5 | Constant |
| Steady: time cap, minutes (0 = none) | `supervisor:steady:minutes` | 0 | Constant (none) |
| Drive: continuations in a row | `supervisor:drive:continuations` | 15 | Constant |
| Drive: time cap, minutes (0 = none) | `supervisor:drive:minutes` | 120 | Constant |
| Stop after N continued turns that used no tool | `supervisor:noprogress` | 2 | Constant, both modes |
| **Wait before continuing**, seconds | `supervisor:cooldown` | **30** | **New** (S5) |
| **Fleet budget**: continuations per hour across all panes (0 = no cap) | `supervisor:hourly` | **30** | **New** (S9) |
| Notify when it needs you (a question, "needs you", "blocked") | `supervisor:notify:needsyou` | on | Today always (via `InputWaiting`) |
| Notify when it hits a limit (continuations, time, no progress, same reply twice) | `supervisor:notify:limit` | on | Today always |
| Notify when it couldn't continue (delivery failed, the agent can't run) | `supervisor:notify:failed` | on | Today always |
| Notify when it finds the work done | `supervisor:notify:done` | on | Today always (`TurnCompleted`) |

- **Not a setting: the same final message twice in a row.** It stops the agent and stays fixed; it is the guard against a loop.
- **Not a setting: the judge model** (Haiku 4.5 through the ambient gateway). The panel states it, with a link to Warden's Ambient section, where the judgements and failures are counted (S6).
- **Wait before continuing** is the one new behaviour and the one that most tames "too active": a continuation is sent this long after the turn ended, and **any turn that starts in that time cancels it**: the operator's Stop or a message, but also a jekt, a job fire or a Schedule injection, so a continuation never lands after a different turn than the one the judge saw (a turn the user starts also resets the run, as before). The row says "continuing in 12 s". A pending continuation is **not persisted**: an srv restart during the wait drops it, and the row says "not continued: AgentMux restarted". **The wait counts toward Drive's time cap**, which is wall-clock time from the run's first continuation. 0 gives today's behaviour. It is applied after the judge's verdict, so the judge still runs once per turn.
- **The notify toggles only narrow the OS notification.** The row, the line ("Needs you: …") and the history show every stop regardless, and the global `notify:os:*` toggles still apply on top.
- **Fleet budget** (S9). Every limit above is per run of one pane, so Drive on eight agents is still 8 × 15 continuations on the operator's account. `supervisor:hourly` caps continuations across all panes in a rolling hour. When it is reached the Supervisor stops continuing, raises the limit notice once (the `limit` group), and each affected row says "fleet budget reached until HH:MM"; agents resume being continued as the hour rolls. In memory, like the run state; a restart starts a fresh hour.
- **Who may change them.** Anyone who can write settings, as for every other key. The 2026-10-25 rule is that agents can do anything except put words in the operator's mouth, so the earlier draft's operator-proof guard on `supervisor:enabled`, on raising a limit, and on `supervisor.set-mode` was dropped (2026-10-26, AgentX): none of them forges the operator's voice. What stays is attribution: srv logs which caller (an agent id, or the app) wrote a Supervisor key through `setconfig` and which called `supervisor.set-mode`, so a change is never anonymous. What stays too is the switch itself: while it is off, `supervisor.set-mode` refuses Steady and Drive for every caller (Off is always accepted).
- **Live, not read at boot.** The supervisor reads each setting from the config when it decides, not from a snapshot taken at start. A test changes a key through the wconfig watcher and sees the next decision use it (this is the one claim in the spec to confirm in the code first; if the watcher doesn't reach the supervisor's reader, phase A adds the hook).
- Limits are checked when a mode runs, so lowering a limit applies to a run already under way at its next turn. Fields are bounded (continuations 1 to 100, minutes 0 to 1440, cooldown 0 to 600, no-progress 1 to 10, hourly 0 to 1000) and show the bound when out of range.

## 7. Edge cases

- **The Swarm pane saved before this** has no `swarm:section`: it opens on the default (Supervisor).
- **A section chosen, then the pane narrowed:** icons only, as for every sectioned pane.
- **Two Swarm panes:** each has its own section; the selection is per `SwarmViewModel`, as today.
- **A broadcast in flight while the section changes:** the composer's state and the result live in the model, not the section, so switching away and back loses neither.
- **A Kanban deep link while no Swarm is open:** opens a Swarm pane on the Kanban section (as `open:swarm` creates one today); with one open in this tab, it switches that pane's section.
- **The Kanban's own state** (polling, filters, lanes) lives in its component and pane meta, mounted once and kept; hiding the section pauses polling through `visible`.
- **Master switch turned off while an agent is mid-run:** the in-flight judgement's verdict is dropped, no continuation is sent, the row shows Off.
- **Settings edited by hand in `settings.json`** take effect the same way; an out-of-range value is clamped and logged.

## 8. Tests

Each written to fail without its change.

- **Sections:** `swarm.test.tsx` (the manifest declares the three sections and `swarm:section`); `sectionOf` default and fallback for Swarm; the title follows the section; the pin menu offers each section; the rail icon-only when narrow.
- **The list is not remounted:** a render test that records the row elements before and after a Supervisor → Broadcast → Supervisor switch and a Kanban round trip, and asserts the same DOM nodes (the #5208 property); the selection survives each switch.
- **Broadcast:** the composer stays docked; the recent-broadcasts list keeps the last 10; the section switch keeps a half-typed message.
- **Supervisor section:** the action bar sets the mode for the selection; disabled with the reason while the switch is off; the chip follows srv's `supports`.
- **Settings:** each control writes its key and shows its default; clamping; the keys exist in the schema and template (the existing schema-sync test).
- **srv:** the master switch gates `evaluate` and `is_supervised`; `set-mode` refuses Steady/Drive while off; the one-time reset clears Steady and Drive once and not twice (per home and channel; a no-op on a pane already Off); a turn started by a jekt, a job or a Schedule during the wait cancels the continuation; a restart drops it; the wait counts toward Drive's time cap; the hourly budget stops continuing and resumes as the hour rolls; a settings change through the watcher reaches the next decision; limits and the no-progress rule read the settings (`policy.rs` tests use a settings value in place of the constants); the cooldown delays a continuation and is cancelled by a Stop or a user message; each notify toggle suppresses only its group.
- **Kanban slot:** `SwarmKanbanSlotProps` is honoured by a stub (visible pauses, openRequest opens and is handled); the held-task chip switches this pane's section; the moved-sections table resolves `warden`/`kanban` in the widget bridge, `sectionProblem`, the pins and `resolve_section`; Warden's rail no longer lists Kanban; a Warden block saved on Kanban opens Host.
- **Live (my own `task dev`, over CDP):** a click on a row in each section; scroll position kept across a Kanban round trip; the Supervisor off banner and a disabled menu; turning it on and setting Drive on a fake-CLI agent with a 30 s wait and a Stop inside it; narrow Kanban in the Swarm at both widths.

## 9. Decisions for the operator

The report's D1 to D7 stand (D1: Supervisor opens by default, now beside Kanban). New ones, each with a recommendation; the rest of the spec assumes the recommendations.

| # | Decision | Recommendation |
|---|---|---|
| **S1** | Rail order | Supervisor, Broadcast, Kanban (the order you named this time; the report had Broadcast first) |
| **S2** | Existing Steady/Drive settings | Reset to Off once, at the first start after this ships |
| **S3** | What Warden keeps of the Kanban | Nothing: no stub. Commands, the widget, the badge and the chips follow the move; old pins and SDK calls keep working through the moved-sections table |
| **S4** | The Kanban section and the agent list | The list stays mounted and hidden while Kanban shows, so no row is ever remounted |
| **S5** | A new "wait before continuing" setting | Add it, default 30 s (0 gives today's behaviour); the one change that makes it less eager |
| **S6** | A judge-model setting | No. It is Haiku 4.5, stated read-only in the panel, with a link to the Ambient section's counts |
| **S7** | Limits and notify toggles | Settings with today's values as defaults (§6.2), no others. "Same final message twice" stays fixed |
| **S9** | A fleet-wide hourly budget | Add `supervisor:hourly`, default 30 continuations an hour across all panes (0 = no cap); AgentX's proposal. If you say no, drop it and the rest stands |
| **S8** | Phase order | Supervisor switch, reset and settings first (§10 phase A), so it is off before anything else ships |

## 10. Phases, owners, files

Each phase is its own PR with a changeset; the spec ships with the first one.

- **A. Supervisor off and settings (srv + settings, Camper).** `supervisor:enabled` and the other keys in `schema/settings.json`, `settings-template.jsonc`, `crates/srv/src/backend/wconfig/types.rs`, `frontend/types/srv-types.d.ts`; `supervisor/mod.rs` (`limits()` reads settings; `is_supervised`), `watcher.rs` (`evaluate`, the cooldown, the notify gates), `policy.rs`, the `set-mode` command in `server/app_api/supervisor.rs`; the one-time reset in `crates/srv/src/migrations/`; tests above. The Settings pane gets a search entry. Without the section UI, the switch is `settings.json` or the Settings pane.
- **B. The scaffolding (Camper).** `swarm.tsx` (`sections`), `swarm-view.tsx` split into `swarm-view.tsx`, `swarm-agent-list.tsx`, `swarm-section-bar.tsx`; `swarm-model.ts` (`sectionAtom` through `sectionOf`); `swarm-kanban-slot.ts` (the props type); the types in `srv-types.d.ts` and `sdk/widget-sdk/v1.d.ts` for `swarm:section`; `defwidget@swarm`. AgentY is told first. Ships with the Supervisor and Broadcast items; the Kanban item absent.
- **C. The sections (Camper).** `swarm-broadcast.tsx`, `swarm-supervisor-section.tsx`, `swarm-supervisor-settings.tsx`; `swarm-fleet-toolbar.tsx` is split between them; the render tests move: `swarm-fleet-toolbar.render.test.tsx`, `swarm-fleet-remote.render.test.tsx` and `swarm-view-remote-selection.render.test.tsx` are split across the two sections.
- **D. The Kanban in the Swarm (Naki-2, #2417), against §3.4.** The slot's lazy mount and rail item, the container and narrow measure, the meta move, the table in §4, `pane-sections.ts`' moved-sections table, `widgets.json`, the badge, the docs, the operator-config text.

B before C and D; A is independent and first. After A the operator can already keep the Supervisor off with confidence.

## 11. Risks

- **Click loss in the list (#5208).** The extraction moves `AgentRow`'s host; the DOM-identity test in §8 is the guard, and AgentY reviews the extraction.
- **File size.** `swarm-view.tsx` is at 1453 lines; the split in B is also what keeps it under the limit. `swarm-model.ts` (2123, in the baseline) must not grow: the section atom and the recent-broadcasts list go in small modules.
- **The wait before continuing** changes how the Supervisor feels once on; at 30 s some Steady/Drive flows will be slower than the operator remembers. It is a setting.
- **Moved-sections table** is a new concept in the contract Naki-2 owns; #2417 owns its shape and this spec only states what it must resolve.

Agent: Camper@narko
