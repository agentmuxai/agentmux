# Report: the Swarm as a sectioned pane (Broadcast, Supervisor), and AgentMux's ambient model calls as a Warden section

**Date:** 2026-10-21
**Author:** QTimer@narko, at the operator's request
**Code read at:** `origin/main` a144c6bf2
**Status:** active. The operator took every recommendation in section 8 (2026-10-21). Phase 1 (section 6, the Warden Ambient section, with the Swarm's Stats panel removed) ships with this report; phases 2 to 4 follow in their own PRs.

## 1. The ask

On 2026-10-21 the operator asked for a better Swarm design, aligned with the other panes:

- Use the formal sections work (sub-panels inside a pane) to give the Swarm two sections, **Broadcast** and **Supervisor**. Each holds the selectable list of agents, but each offers different actions.
- Move the stats out of the Swarm.
- The lines the Swarm shows are probably shared elsewhere, or should be generalized. If so, they would be monitored from Warden: the stats are really stats about AgentMux's ambient activity, which can become one Warden section.
- Find who wrote the formal widget and sections work.

## 2. Short answers

- **Who wrote the sections work:** Naki-2@narko (`naki-naki-2`), in `docs/reports/REPORT_PANE_SECTIONS_AS_WIDGETS_AND_VAULT_IN_WARDEN_2026_10_21.md`, built in #5284 (sections are part of the pane contract), #5286 (the Vault is Warden's Vault section), #5288 (pin a pane's section to the widget bar) and #5279 (what meta a widget may pass). It extends Camper@narko's `docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md` (the `PaneTabManifest` contract, from #3723 on). The widget SDK side is lark@narko's `docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md`. Note that the sections contract lives in a report, not a spec.
- **The Swarm doesn't use sections.** It is one view with a toolbar of actions (Select all, Broadcast, Stop, Supervisor, Stats, Clear) over one long agent tree. It is the only large multi-purpose pane that hasn't moved onto the sections contract.
- **The lines are already shared**, but they are named after the Swarm. The line under each row comes from `frontend/app/store/swarm-line.ts` and `activitySummary.ts`. It is also the pane tab's tooltip, the agent pane's footer and bottom panels, and the line other machines show for this machine's agents (srv `backend/agent_line.rs`). It should be renamed to an agent line, not a Swarm line (section 5).
- **The stats are not message counts.** The Stats panel counts how **AgentMux's own model calls** ended (the `crates/srv/src/ambient/` module: session titles, prompt suggestions, names, previews, narration, running summaries, Supervisor verdicts). Those calls are what write the lines. So "monitor the lines from Warden" and "stats for ambient activity" are the same thing, and they make one Warden section: **Ambient** (section 6). Counts of messages between agents (jekts, broadcasts) don't exist anywhere today; section 6.4 covers that.
- **Recommendation:** first a Warden **Ambient** section, which is small and independent. Then the Swarm split into **Broadcast** and **Supervisor** on the sections contract, with one shared agent list and selection. Then rename the line module.

## 3. Today

### 3.1 The Swarm pane

Files: `frontend/app/view/swarm/` (`swarm-view.tsx` 1453 lines, `swarm-model.ts` 2123, `swarm-view.scss` 1192, `swarm-fleet-toolbar.tsx`, `swarm-stats.tsx`, `swarm-supervisor.tsx`, `swarm-remote.ts`, `swarm-fleet-targets.ts`, others). Registered as `swarmPaneTab` in `swarm.tsx:10` with no `sections`.

Top to bottom (`swarm-view.tsx` `SwarmView`):

| Part | What it does |
|---|---|
| `FleetToolbar` (`swarm-fleet-toolbar.tsx`) | Select all / none; "N selected · M on other machines"; **Broadcast** (an inline composer that replaces the buttons); **Stop N** (confirm modal, staged rollout from 5 targets); **Supervisor** (a menu of the three modes); **Stats** (toggles the panel, shows "N failing"); Clear |
| `SwarmStatsPanel` (`swarm-stats.tsx`) | Slides down under the toolbar: "AgentMux's own model requests since its server started", one row per purpose |
| `FleetResultPanel` | The last fleet action's result, per target |
| Empty state, "Clear completed (N)" | |
| `swarm-tree` | One row per local agent (`AgentRow`), then the other instances' agents (`RemoteAgentRows`) |

Each local row has a checkbox, provider icon, name, unseen-turns dot, context size, last active, status chip, `SupervisorChip`, the summary line, the held Kanban task chip and the current tool. It also has child buckets: Supervisor decisions, Todos, Agent tools, Workflow, Shell, Cron, Long-running and Background.

Selection is `selectedBlockIdsAtom` in `swarm-model.ts` (local block ids and remote keys). Broadcast goes through `FleetBroadcastCommand` to srv `fleet.broadcast` (`crates/srv/src/server/app_api/fleet.rs`). The Supervisor modes (`off`, `steady`, `drive`) are set through `supervisor.set-mode`, and the judge lives in `crates/srv/src/backend/supervisor/` (`docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md`).

The problem the operator sees: the actions are a toolbar that changes shape with the selection, and the composer hides the other buttons. The stats are a fold-down panel. Two quite different jobs, talking to many agents at once and keeping an eye on them, share one row of buttons and one list.

### 3.2 The sections contract (Naki-2)

- `frontend/app/block/pane-sections.ts`: `PaneSections` (`default`, `items: [{id, label, icon, sections?}]`). The selected section is in block meta under `<view>:section`. Helpers: `sectionOf`, `sectionProblem`, `sectionMetaOf`.
- `PaneTabManifest.sections` (`pane-tab-registry.ts:166`). Declared today by Warden, Settings, Connectors and Memory; the Vault is a nested section of Warden.
- A generic `SectionPaneModel` / `SectionPaneView` (`frontend/app/view/section-pane/`). Connectors and Memory use it; Warden and Settings keep their own views but declare their sections.
- What a pane gets for free once it declares sections: `widgets.json` entries with `"section": "<id>"`, validated; "Pin “<section>” to bar" in the pane header's menu (#5288); a typed section in the widget SDK's `am.panes.open`; a title that can follow the section.

### 3.3 Warden

Seven sections in `frontend/app/view/warden/warden-sections.ts`: Kanban, Vault, Host, LAN, Internet, Audit, History (default Host). They are rendered by `warden-view.tsx` through `TabbedPane`, with every section mounted and hidden when not selected. History, Kanban and the Vault load only when first shown, through a `visible` prop. Adding a section touches the union and list in `warden-sections.ts`, a pane block in `warden-view.tsx`, a `frontend/app/view/warden-<name>/` folder, `frontend/types/srv-types.d.ts` (the `warden:section` union) and `sdk/widget-sdk/v1.d.ts`. Optionally it also adds a palette command (`open:kanban` pattern) and a widget-bar entry (`defwidget@kanban` pattern).

## 4. Proposal: the Swarm as two sections

### 4.1 Shape

- `swarmPaneTab` declares `sections: { default: "supervisor", items: [broadcast, supervisor] }`, with meta key `swarm:section`. It is rendered with the same top `TabbedPane` as Warden, so it looks and behaves like the other sectioned panes. That gives it icons only when narrow, labels when wide, the same zoom, and a title that follows the section ("Swarm · Broadcast").
- Rail order as the operator named them: **Broadcast** (`tower-broadcast`), **Supervisor** (`user-shield`).
- Default: **Supervisor**, because watching agents is what the Swarm is opened for most (decision D1).
- Pinning comes free from #5288: someone who broadcasts often can pin "Broadcast" to the widget bar.

### 4.2 One agent list, one selection

Both sections show the same agents (this instance's, then other instances' and LAN peers'), from the one `SwarmViewModel`, so there is still one poll and one set of event subscriptions. The selection is shared: agents selected in Broadcast are still selected when you switch to Supervisor, and the count stays visible in both (decision D2).

**The list is rendered once, not once per section.** The section switch changes only the action area (the composer, or the Supervisor bar) and a `density` the rows read (compact or full). The rows themselves are never remounted by it. This keeps AgentY's fix from #5208 intact (`docs/specs/SPEC_PANE_SELECTION_STATE_MACHINE_AND_SWARM_CLICK_2026_10_18.md`). Rows are keyed by block id through `AgentRowHost`, `AgentRow` takes its node as an accessor, and a mouse press selects. The card and its label elements must stay the same elements across updates, because a click is lost when the pressed element is replaced before release. Two mounted copies of the list, as Warden mounts every section, would double that work and risk the same loss. The list becomes a component of its own, `SwarmAgentList`, taking `density` and the selection. Hidden detail, such as the child buckets in compact mode, is collapsed, not unmounted, wherever that keeps the row's own elements stable. The select tracing from #5193 (`frontend/app/util/select-trace.ts`, the `traced()` setters in `swarm-model.ts`) stays as it is.

### 4.3 Broadcast section

- **The list:** compact rows: checkbox, provider icon, name, status chip and the agent's line. No child buckets: you are choosing recipients here, not inspecting.
- **The composer is always there**, docked under the list (a multi-line field, Send, the recipient count and the "N can't receive this" note from `gapSummary`). It replaces today's inline composer, which hides the other buttons while open.
- **The result:** each broadcast's per-target result (today's `FleetResultPanel` for `broadcast`) shows under the composer.
- **Optional, new:** the last few broadcasts sent from this Swarm (text, when, recipients, delivered or failed), from the result entries the model already holds. They are not persisted (decision D5).

### 4.4 Supervisor section

- **The list:** today's full rows, with the status, the Supervisor chip, the summary line, the held Kanban task and every child bucket (Supervisor decisions, Todos, tools, Workflow, Shell, Cron, Long-running, Background). This is the monitoring view.
- **The actions,** in a bar above the list:
  - set the Supervisor mode for the selection (`off` / `steady` / `drive`);
  - **Stop N**, with its confirm and staged rollout;
  - Clear completed.
  - Stop moves here because stopping an agent is oversight, not messaging (decision D3).
- **Remote agents** show as today. An action they can't take says why, through `unavailableReason`; the Supervisor only runs for this AgentMux's own agents.

### 4.5 What leaves the Swarm

The Stats button, its "N failing" badge and `SwarmStatsPanel` go to Warden (section 6). The model's `ambientOutcomesAtom`, `loadAmbientOutcomes` and `statsOpenAtom` leave `SwarmViewModel`. Whether the Swarm keeps a small "2 failing" link that opens Warden's Ambient section is decision D4.

### 4.6 Code

- `swarm.tsx` declares `sections`. `swarm-view.tsx` gets the `TabbedPane` section row and one `SwarmAgentList` under it. The two sections are action areas, not two lists (4.2). The 1453-line view is split: `swarm-agent-list.tsx` (the rows, unchanged in how they are keyed and how they select), `swarm-broadcast.tsx`, `swarm-supervisor-section.tsx`.
- `swarm-fleet-toolbar.tsx` splits into the Broadcast composer and the Supervisor action bar. `FleetResultPanel` is shared.
- `SwarmViewModel` stays one model. `sectionAtom` reads `swarm:section` through `sectionOf`, as Warden's does.
- `frontend/types/srv-types.d.ts` and `sdk/widget-sdk/v1.d.ts` gain `swarm:section`. `defwidget@swarm` stays as it is and opens the default section.
- Tests: the section switch keeps the selection; each section's actions act on the selection; `sectionProblem` accepts `swarm` with `broadcast` and `supervisor`; the pin menu offers both.

## 5. The line: shared already, named after the Swarm

What a row shows under an agent is resolved by `resolveSwarmLine` (`frontend/app/store/swarm-line.ts`). In order, it uses the generated title, the last session's title, a one-liner from the last prompt, or a status phrase; "Waiting for you" wins while a turn is in flight. Its base, `readSwarmSummary` (`activitySummary.ts`), and its meta keys are also used by:

- `frontend/app/element/PaneChrome.tsx`: the pane tab's tooltip;
- `frontend/app/view/agent/components/AgentBottomPanels.tsx` and `AgentFooter.tsx`;
- the agent hooks `useAgentActivitySummary.ts`, `useBlockActivity.ts`, `useAgentQuestions.ts` and `awaiting-user.ts`;
- srv `crates/srv/src/backend/agent_line.rs`, which sends the line to this machine's other channels and to LAN hosts, under the setting `swarm:sharesummaries`.

So it is already the agent's line, used in four places, and only its names say Swarm. Recommendation: rename `swarm-line.ts` to `agent-line.ts` (`resolveAgentLine`, `readAgentSummary`) in one mechanical PR, and leave the setting key `swarm:sharesummaries` alone, because renaming a stored setting needs a migration for no visible gain. Only its label in Settings changes, to "Share what my agents are working on" (decision D6).

The line is written by ambient model calls (session titles, and title recovery when one is missing). So the way to monitor whether lines are healthy is the Ambient section's numbers for those purposes. That is the "monitored from Warden" the operator described.

## 6. Proposal: a Warden "Ambient" section

### 6.1 What it shows

One section, **Ambient** (icon `wand-magic-sparkles`, after History), titled "AgentMux's own model calls". It has one row per purpose, in the order of `PURPOSE_NAMES` (`frontend/app/store/ambient-outcomes.ts`):

| Column | Source |
|---|---|
| Purpose (Session titles, Session titles (recovery), Prompt suggestions, Subagent names, Workflow names, Agent previews, Narration, Running summaries, Supervisor verdicts, and any purpose this build doesn't name) | `ambient.outcomes` |
| Requests, accepted, kept, refused, skipped, failed; the raw labels in the hover | `ambient.outcomes`, summarized by `summarizeAmbientOutcomes` (already outside the Swarm) |
| Tokens spent | the `ambient:spent` event (`crates/srv/src/ambient/spend.rs`), which carries the purpose. Today `token-usage.ts` files all of it under one `__ambient__` bucket for the status bar, so keeping the per-purpose split is new frontend work |
| A "failing" mark on a row | `unhealthy` (refusals and failures outnumber useful answers) |

The counts run from srv's start and reset when it restarts; the section polls every 5 seconds while shown, as Host, LAN and Audit do. *As built:* the tab rail has no badges and Warden polls no section that isn't shown, so the failing count is in the pane's title while the section is shown ("Ambient · 2 failing"), the way Kanban's title carries "N need you". Tokens per purpose are left for phase 4.

### 6.2 How it is built

- Add `ambient` to the `WardenSection` union and `WARDEN_SECTIONS`, to `srv-types.d.ts` and to the SDK typings.
- Add `frontend/app/view/warden-ambient/warden-ambient-manager.tsx`. It takes a `visible` prop like History's: it loads when shown and polls while visible (the Swarm used its 12-second poll).
- Move the row rendering out of `swarm-stats.tsx` and its styles out of `swarm-stats.scss`.
- Optional palette command `open:ambient` (the `open:kanban` pattern).
- No srv change for the counts. Per-purpose spend needs only the frontend change above.

### 6.3 Docs and comments to update with it

- `SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md` §5.8 says the counts moved to the Swarm on 2026-10-04; add that they moved to Warden.
- `crates/srv/src/ambient/outcome.rs` line 9 still says the counts show "in the Instance panel"; that is already wrong.
- agentmux-docs' Warden page (Naki-2's report §6.2 already lists it as stale).

### 6.4 "Ambient messages": counts of agent-to-agent messages

If the operator meant messages between agents as well (jekts, Swarm broadcasts, `[AgentMux]` notes, held deliveries), nothing counts them today. Warden's Audit section lists the last jekt deliveries (`GET /agentmux/reactive/audit`, `audit/jekt-audit.jsonl`), but there are no totals, and held jekts and held notes aren't exposed at all. Adding counters means new srv work: counts per kind and outcome (delivered, held, failed), kept the way `ambient/outcome.rs` keeps its own, and one RPC. That would be a second table in the same section, or a summary at the top of Audit. Recommended as a later phase, once the Ambient section exists (decision D7).

## 7. Order

1. **Warden Ambient section**, with the Stats panel removed from the Swarm. Small, independent, one PR.
2. **The Swarm on sections:** Broadcast and Supervisor, with the shared list and the split toolbar. One PR, or two if the list extraction goes first.
3. **Rename the line module** to `agent-line.ts`. Mechanical, any time.
4. **Optional:** per-purpose ambient spend; message counters (6.4); recent broadcasts (4.3).

Each needs a changeset. The existing render tests move with the code: `swarm-stats.test.ts` and `swarm-stats.render.test.tsx` go to the Warden section, and `swarm-fleet-toolbar.render.test.tsx`, `swarm-fleet-remote.render.test.tsx` and `swarm-view-remote-selection.render.test.tsx` are split across the two sections. Each Swarm PR should also be tried by hand in a `task dev` build.

## 8. Decisions for the operator (recommendation first)

**Decided 2026-10-21:** the operator took every recommendation below.

| | Question | Recommendation |
|---|---|---|
| D1 | Which section opens by default? | Supervisor; Broadcast can be pinned to the bar |
| D2 | One selection shared by both sections, or one each? | Shared |
| D3 | Where does Stop live? | Supervisor |
| D4 | Does the Swarm keep a small "N failing" link to Warden's Ambient section? | No: Warden's title shows the count (6.1) |
| D5 | Recent broadcasts in the Broadcast section? | Yes, in memory only, the last 10 |
| D6 | Rename the line module, keep the `swarm:sharesummaries` key? | Yes, and yes |
| D7 | Count agent-to-agent messages too (6.4)? | Later, as its own phase |

## 9. Who to involve

- **Naki-2**: the sections contract and pinning; the Swarm becomes its sixth sectioned pane.
- **Camper**: the Supervisor (#5036), the Stats panel (#4330), the pane-tab contract.
- **Lazo**: the Swarm's one-list and remote-broadcast work (`SPEC_SWARM_REFINEMENTS_2026_10_17.md`).
- **AgentY**: Swarm row click and selection (`SPEC_PANE_SELECTION_STATE_MACHINE_AND_SWARM_CLICK_2026_10_18.md`, #5208, #5193). AgentY has tasks #158 (a selection state machine that owns `revealBlock`) and #164 (route the other callers through it) queued. The redesign doesn't change how a row is built or selects (4.2), so those don't need to wait. Tell AgentY before any PR that touches `AgentRow` or `AgentRowHost`.
- **AgentX**: the ambient hardening spec.
- **Loap-3**: the Kanban in Warden and its task chip on Swarm rows.

At the time of writing nobody had claimed the Swarm or Warden folders (Presence).
