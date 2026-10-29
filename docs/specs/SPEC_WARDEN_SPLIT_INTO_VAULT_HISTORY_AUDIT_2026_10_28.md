# Spec: Warden goes; Vault, History and Audit become widgets of their own; Jobs go to Drone

**Task:** #9204 (spec, revised); build #9205 follows approval.
**Author:** Loap@narko, 2026-10-28.
**Status:** active: approved by the operator 2026-10-29 ("warden split spec is good, proceed with it", relayed by AgentY); being built in phases (§9). Phase 1's first part, the remaining server paths that took a history reader for the agent's pane, is in the PR that adds this file (#5776). Revision 2: replaces the first version of this spec (`SPEC_WARDEN_RETHINK_2026_10_28.md`, which kept Warden with five sections), after the operator changed direction.
**Direction (the operator's words, relayed by AgentY, verbatim):** "The Jobs should really be part of drone. we'd want each of the items, when selected, show the corresponding drone diagram. whatever the job is, there is a diagram that describes it. ...Meanwhile Audit needs to be beefed up, for one, can a jekt open the exact agent pane? we also need to audit agent start, agent end, and every tool call and thought. can they all get links? I am thinking more we should have top level for vault, audit, history. Ambient can go into the status bar pill for tokens. that takes care of allof them right? so we will just drop warden, and add 3 widgets in place: Vault, History, and Audit. Jobs goes to drone, host is already covered by swarm, LAN is covered in the status bar pill, Internet goes away, and ambient goes into the status bar token info"
**Verified against:** `agentmux` `origin/main` @ `9db194787` (2026-10-28) by reading the code named below. Nothing was run. Where a claim rests on a file I did not open, it says so.
**Builds on:** [SPEC_JOBS_AND_TEMPLATES_2026_10_24.md](SPEC_JOBS_AND_TEMPLATES_2026_10_24.md) (the Jobs ledger), [SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md](SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md), [SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md](SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md), [SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md](SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md), and Camper's singleton-flag spec for #9109 (a draft in Camper's workspace, not committed: `SPEC_WIDGET_SINGLETON_FOCUS_OR_NEW_2026_10_28.md`).

## 0. Summary

Warden stops being a pane. Each of its eight sections goes where it belongs:

| Warden section (today) | Goes to | Notes |
|---|---|---|
| Vault | **Vault** widget (own pane, `view: "vault"`) | Its own sections (Cards, Logins, ...) become the pane's sections |
| History | **History** widget (own pane, `view: "history"`) | Agents, Typed, Storage (with Reclaim) as now |
| Audit | **Audit** widget (own pane, `view: "audit"`), **much larger** (§4) | Every entry links to where it happened; adds agent start, agent end, tool calls, thoughts |
| Jobs | **Drone** (§3) | A job list beside the canvas; selecting a job shows the diagram that describes it |
| Ambient | **Status bar token pill** popover (§6) | Into its existing "AgentMux internal" row |
| Host | **Swarm** (§5) | A badge on the agent's row and one row-menu action |
| LAN | **Network pill** popover in the status bar (§5) | Peer detail; re-pinning stays in Settings > Devices |
| Internet | Removed | It was a stub |

The Kanban already left (it went to the Swarm, and on 2026-10-29 from there to a pane of its own, `view: "kanban"`, #5771). After this nothing is left of Warden: no pane, no widget, no `view: "warden"`. Old saved panes, widget-bar pins, `widgets.json` entries, palette entries, SDK calls and layout files that name it all keep working through one redirect table (§7).

Three widgets, three singletons (per Camper's flag): **Vault**, **History**, **Audit**. **Drone** is added to the singleton list too, since it now holds every job (§1).

## 1. The three widgets

| Widget | View | Icon | Singleton | Sections |
|---|---|---|---|---|
| Vault | `vault` | `lock` | yes | the Vault's own (`VAULT_SECTIONS`), meta `vault:section` |
| History | `history` | `clock-rotate-left` | yes | a segmented control as now: Agents, Typed, Storage |
| Audit | `audit` | `list-check` | yes | none: one filtered list (§4) |

- **Panes, not sections.** Each is a native pane tab, registered in `block-registry.ts` the way `wardenPaneTab` is. The `vault` view existed before it became a Warden section, and its blocks still say `vault` (Warden registers `aliases: ["vault"]` for that). The alias goes: `vault` is a real view again. Its code (`vault-view`, `VaultViewModel`) already has its own model; `vault-in-warden.ts` (the adapter that lets Warden host it) goes, and the Vault's protections ("counts as shown only while selected") become "while the pane is visible".
- **Bar and menu.** Entries are `defwidget@vault`, `defwidget@history`, `defwidget@audit` in `crates/srv/src/config/widgets.json`, next to where Warden and Vault were (`display:order` 15 and 16, plus one: the build checks 17 is free). `defwidget@warden` is deleted. None is pinned by default (Warden and Vault were not either): they are in the More menu, and pinning is one right-click. (Question Q2.)
- **Singleton (#9109).** `"singleton": true` on all three. Camper's draft table lists "Warden (and its Vault entry)" as a singleton; it must read "Vault, History, Audit and Drone" instead, and Drone, which is in neither column of the draft, goes in the singleton one: there should be one place for jobs, not several. Camper (Swarm and widget-bar owner) is told; #9110 is the build that adds the flag.
- **Titles.** The pane title is the widget's name (`Vault · Cards`, as now); Drone's title carries "N failing" (§3), and the Warden-specific "Ambient · N failing" title goes with Ambient (§6).
- **Code that moves.** `frontend/app/view/warden-history/` becomes `history/`; `warden-audit/` becomes `audit/`; `warden-jobs/` becomes `drone/jobs/` (§3); `warden-shared/` (the manager chrome and helpers) is renamed `pane-manager-shared/` for what remains (History, Audit, Drone's job list); `warden-host/`, `warden-lan/`, `warden-internet/`, `warden-ambient/` and `warden/` are deleted. `warden-kanban/` is renamed by the Kanban pane's own work (#5771 gave it its own view; check whether the directory moved with it). The generated names `warden:section` and the `WardenSection` type go from `srv-types.d.ts` and `sdk/widget-sdk/v1.d.ts`.

## 2. Why this is simpler than it sounds

Warden was a shell that mounted its sections lazily, hosted the Vault's model for it, owned a rail and a title, and told each section when it was visible. Each of those jobs is now a pane's own: three panes, each with one body. What goes is `warden-view.tsx`, `warden-model.ts`, `warden-sections.ts`, `warden.tsx`, `warden-instance.tsx`, `open-warden.ts` and `vault-in-warden.ts`. What is added: three small pane-tab manifests, the Drone job list, the Audit query, and the redirect table.

## 3. Jobs go to Drone

### What the Jobs section is today

`warden-jobs/` (about twenty files): `JobsModel` (`jobs.list`, refreshed on `jobs:changed` and every 15 s while shown), a list of `JobView`s (name, source `cron` or `drone`, owner, trigger, next run, state, recent runs, cost today and this week, over-cap, an open incident), per-job History, **Run now**, **Pause**, **Resume**, **Delete**, **Pause all**, the job editor with a dry run, job templates and their approval, and the cap defaults. `JobView.source` is already `cron | drone`; templates have a `kind` of `job | cloud`.

### What Drone is today

`frontend/app/view/drone/` is one canvas: a node-type bar on top, a name field, Save and Run, the graph (nodes of five kinds: `agent`, `condition`, `api`, `response`, `variables`; wired by ports), an inspector and a run-state overlay. It edits one definition at a time (`ListDrones`, `GetDrone`, `UpsertDrone`, `RunDrone`, `ListDroneRuns`); a Drone run is **manual only** today, and since Jobs P1 each run lands in the shared run ledger. Drone has no sections and no job list. `drone-view.tsx` is 1,046 lines and `drone-model.ts` 627, against a 1,500-line limit. I read the class names in `drone-view.tsx` and the model's methods, not its layout code line by line.

### The design

1. **A job list beside the canvas.** Drone gains a left rail, "Jobs", collapsible, holding what the Jobs section holds today: every job with its state, next run, last outcome and failing mark, grouped (Running by itself, Paused, Cloud, AgentMux's own). **Selecting a job shows its diagram in the canvas** and its detail (history, cost, incident, Run now / Pause / Resume / Delete) in the inspector area the canvas already has for a node's fields and a run's state. A Drone graph is a job whose diagram is its own graph, editable as now. "New" offers a job from a template, a cron job, or an empty graph. **Pause all** sits in the rail's header. The title reads "Drone · 2 failing" when jobs fail, as Jobs' did.
2. **Every job has a diagram.** Drone graphs have theirs. For any other job, the diagram is **derived** from the job's own fields, on the fly and never stored, so it cannot drift from the job: a job has a *trigger* and an *action* and a *policy* (Jobs spec §2), and the diagram draws exactly those.

   | Job | Diagram (left to right) |
   |---|---|
   | **Cron**, `every`, `once`, manual job that messages an agent | `Trigger` (the cron expression read in its time zone, or "every 15 min", or "once at ...", or "manual"; next fire) → `Agent: <target>` (the message, shortened) → `Result` (where it lands: the agent's pane, the run ledger) |
   | A job that files a Kanban task | `Trigger` → `Kanban task` (title, target) → `Result` |
   | A job that spawns a worker with a brief | `Trigger` → `Spawn worker` (the brief) → `Result` |
   | A job whose action is a Drone graph (Jobs P4) | `Trigger` → `Run graph: <name>` (a node that opens the graph; double-click drills in) → `Result` |
   | **Cloud job** (runs elsewhere, reports in over MuxBus) | `Trigger` (its schedule in the cloud) → a dashed box "outside this computer" holding `Runs in the cloud` → `Reports in` (run events) → `Result`, with a `Watchdog` node: "expected by T, otherwise recorded as missed and alerted" |
   | **AgentMux's own scheduled work** (see below) | `Trigger` (a timer or an event) → `Action` (what it does) → `Shows up in` (the Swarm line, the pane tab tooltip, ...) |

   The **policy** (catch-up, overlap, timeout, concurrency group, caps) is drawn as small badges on the `Trigger` and `Action` nodes, not as extra nodes: it modifies them. The last runs are drawn as coloured dots on the `Result` node (the same recent runs the list has), so the diagram also says how the job is doing. A failing job's `Action` node has the failing colour.
3. **What is new in the canvas.** Derived diagrams use the existing look of a node and an edge. The only additions are **display node kinds**: `trigger`, `result`, `task`, `spawn`, `graph`, `cloud`, `watchdog`. They are not part of Rust's `BlockKind` (which the executor dispatches on and which is generated from the engine); they exist in the view layer only, are read-only, have no ports to edit, and are never saved. A Drone graph keeps its five kinds. The derived graph is a pure function `jobDiagram(job): DroneGraph-like` with a test per job type, so adding a new action later adds one case.
4. **Editing.** A derived diagram is read-only. A cron job is edited in the job editor form (as in Jobs today), opened from the inspector area with a pencil; saving redraws the diagram. Converting a cron job into an editable Drone graph needs Drone to start from a trigger, which is Jobs P4 and not part of this spec; until then the honest label on the diagram is "Derived from the job; edit it in the form".
5. **AgentMux's own scheduled work.** Today the Jobs list holds only cron and Drone jobs. The Jobs spec §1.1 lists other things AgentMux runs by itself: the Swarm Supervisor (a finished turn), the runtime watchdog (a 60 s timer), the Ambient purposes (session titles, names, suggestions, narration, running summaries), the held-jekt retry, history compaction. I recommend a small, fixed, **read-only** group "AgentMux's own" in the rail, each with a derived diagram from a static descriptor in the code. Its value is the diagram answering "what is this thing that runs by itself and writes that line under my agent". It is the one piece that adds data, not a move, so it is its own phase (§9, phase 5). Ambient's health table is **not** here: it goes to the token pill (§6), where the cost already is, and an Ambient entry's diagram links to it.
6. **Moves.** `JobsModel`, the editor, dry-run, templates, approval and history components move into `frontend/app/view/drone/jobs/` with their tests; their data (`jobs.*` RPCs, `JobsChanged`) does not change. The incident text that says "Warden > Jobs" (§7) changes to "Drone > Jobs". The Jobs `visible` polling becomes "while the Jobs rail is open and the pane is visible".

### Owner and coordination

The roster has no seat for Drone. The code's history: AgentX wrote the canvas spec (2026-06-05) and built Jobs P1 (#1864, which is Jobs' own build); Maricon@charlie last changed it (#5341, zoom); Agent1 touched it for themes. **AgentX is the person to coordinate with and to name an owner for the build**; Agent1 should be told for the canvas styling, and the UI kit owner (Agent1) for the list rail. AgentY owns the Jobs ledger side (`jobs.*`, incidents).

## 4. Audit, beefed up

### What the Audit shows today (and its limit)

`warden-audit-manager.tsx` lists the newest 50 entries of `GET /agentmux/reactive/audit`, which reads **a 100-entry in-memory ring** (`AUDIT_LOG_MAX = 100` in `backend/reactive/mod.rs`), emptied at every restart. An entry (`AuditLogEntry`, `reactive/types.rs`) holds: time, `source_agent`, `target_agent`, `block_id`, a **hash** and the **length** of the message (never its text), success and error, a `request_id`, an `outcome`, a `reason`, an `event_kind` (`delivery`, `action`, `register`, `unregister`, `swept`, plus `cli_native` for CLI-native messages), the sender's attributed UID and the WAN verdict.

But every entry is **also** appended to a durable file, `<shared root>/audit/jekt-audit.jsonl` (`reactive/audit_store.rs`), which "is never rotated or truncated" and is shared by every srv on the machine (each line names its channel). So the record already exists forever; the pane just reads the 100-line ring instead of the file.

### What "link" can mean here: three facts that make it feasible

1. **A jekt's id is findable in the receiver's transcript.** The `MSGID=` in the marker line every agent receives is the audit entry's `request_id` (`handler.rs`: `request_id` is passed to `wrap_jekt_message_with_files` as `msg_id`). So the audit line names exactly the text to find: `MSGID=<request_id>` in the receiver's recorded conversation, and the sender's `Message` tool call (which returns `id=...`) in the sender's.
2. **A position in a transcript is a byte offset, and the history reader already jumps to one.** The transcript is a line-indexed file per agent (`output`, `output.idx`), with a time sidecar (`output.tsidx`: `{off, ms}` per append, binary-searched in `shell/stamps.rs`). The History reader's `HistoryNavigator.locate({ at_ms | at_byte })` resolves either to a line and scrolls there (#5159). Compression keeps logical offsets (`HistoryCompression` carries `logical_bytes` apart from `stored_bytes`); the build confirms Reclaim never rewrites them.
3. **Agent start and end are already recorded.** The segment ledger (`backend/continuity_segments.rs`, an append-only `segments.jsonl` per agent UID, also kept forever) writes `start` at spawn (time, provider, working directory, channel, block, the byte where its conversation starts) and `end` when the process goes (time, **reason**: `exited`, `crashed`, `closed`, `restarted` or `stopped`, and the byte where it ended). That is "agent start, agent end (with reason)" with positions, today, without new writes.

What does **not** exist: an index of tool calls and thoughts. They are lines inside the transcript (for Claude, `tool_use` and `thinking` blocks of the stream-json), found only by reading it.

### The model: an index over three stores, not a copy

The Audit does not copy content. It is a merged, filterable view over what each store already holds, plus one new index:

| Entry kind | Lives in (source of truth) | New? | Link target |
|---|---|---|---|
| **Jekt** (delivered, refused, held, retried, system note), CLI-native message | `jekt-audit.jsonl` | no (read the file, not the ring) | Receiver and sender, each at the message (below) |
| **Action** (fleet or pane command, with its reason), **register / unregister / swept** | `jekt-audit.jsonl` | no | The acting agent's pane; the target pane if it still exists |
| **Secret use** (an agent filled a web form or ran a command with a Vault entry) | the Vault's activity log (`loadActivity`, what `WardenSecrets` in `vault/vault-warden.tsx` lists today; never a value) | no | The agent that used it, at the call. The "where agents may use secrets without asking" grants, with Revoke, stay in the Audit above the list (they are about what agents did) |
| **Agent start / agent end** (+ reason) | the segment ledger | no | The agent's history at the segment's first or last byte; its pane if open |
| **Tool call**, **thought** | **a per-agent event index**, a sidecar beside `output.tsidx` (`output.evidx`): one record `{off, ms, kind, tool, len}` per tool call or thought, written at the same moment and in the same transaction as the transcript append | **yes** | The agent's history at `off` |

The index holds **no text**: a kind, the tool's name, a length and the offset. The reader shows the text from the transcript itself, so the Audit adds no copy of anything a person typed, a model thought or a tool returned. Sizes: an index record is a few dozen bytes against transcript lines that are kilobytes; the existing time sidecar of a long-lived agent holds about 1.1 million records in 38 MB (`stamps.rs`'s own measurement), which is the order of magnitude for the busiest agent, not for the fleet.

**Retention is the history's: forever.** The audit file already is. The segment ledger is. The event index lives and dies with its transcript zone (including archives), so it can never point at text that is gone, and "Delete this agent's history" deletes its index with it.

**Providers.** The index is written by the history adapter (`history/adapter.rs`, `claude_adapter.rs`) that already classifies a provider's lines. Claude's tool calls and thinking are recognised first; a provider whose stream the adapter cannot classify yet has start, end, jekt and action entries, and no tool-call or thought entries, and the Audit says so ("not recorded for this provider") instead of showing a gap. Already-recorded history: backfill lazily, one agent at a time, when that agent is first filtered (a bounded scan), instead of re-reading every transcript at once (Question Q5).

### One query, server side

`audit.query` (new RPC; the pane never reads the files):
`{ kinds?, agents?, from_ms?, to_ms?, text?, cursor?, limit }` → `{ entries, next_cursor }`. srv reads the three stores in time order, merges them (they are each ordered by time) with a cursor, applies the filters, and returns a page. The pane keeps a virtual list and asks for the next page as you scroll. **Filters:** agent (several), kind (chips), time range, and a text box over the entry's summary (tool name, sender or receiver, reason). **Volume:** tool calls and thoughts are the bulk, so they are **off by default**: the default view is jekts, actions, starts, ends and failures, and tool calls and thoughts appear when you tick them or filter to one agent. A page is a bounded read of each source, never a scan of a transcript.

### Every entry has links

Each row has up to two small link buttons, shown only when they can work:

1. **In history** (always, while the history exists): opens the History reader on that agent **at the entry's position**: a jekt at the message, a start or an end at the segment's byte, a tool call or thought at `off`. It uses the reader from the History fix (§8): read-only, never a live pane, works whether the pane is open, closed or the agent deleted.
2. **In its pane** (while the pane is open): reveals the agent's live pane (`revealBlock`, which already crosses tabs and windows, #9107 makes it restore a minimised one). A jekt row has this for **both ends**: "sender's pane" and "receiver's pane". Today the live pane can be revealed, not scrolled to a past position (only the history reader has "jump to date or byte"); scrolling a live pane to an old position is a new build item, in the Audit's phase 2. Until it lands, "In its pane" reveals the pane and "In history" does the exact positioning.

So the operator's question, "can a jekt open the exact agent pane?": **yes to the pane, today (the audit line carries the receiver's `block_id`); to the exact message in that pane, through the history reader now and in the live pane after the scroll-to-position item.**

The Swarm gains one entry point: a row's menu "Audit for this agent" opens the Audit pre-filtered to that agent.

### Cost and ownership

The three small changes in srv: the `audit.query` RPC and its merge; reading `jekt-audit.jsonl` with a cursor and filters (it is a plain JSON-lines file, newest at the end); the event index written by the history adapter. Agent5 owns the history store and its indexing (`shell/indexing.rs`, `stamps.rs`, `history/`), AgentY the jekt audit (`reactive/`); the Audit pane itself is the build owner's. The Audit stays an operator-facing view; the entry kinds carry no message text, so nothing in it is more sensitive than what the audit file already holds.

## 5. Host and LAN

### Host goes to the Swarm (the badge design stays)

Host showed, per registered agent, its name, block, last seen and active/idle, with an expansion for "Also claimed by another instance on this host" and "Recent identity mismatch", and a Deregister button. Everything except the last three is on the Swarm row already. The three that are not move there:

1. **A row badge**, `am-warn` style like `swarm-row-notes.tsx`: "Another instance claims this agent" or "A message to it was refused: identity mismatch", with the same detail on hover or click (channel and pid, the time, the block, the error text, and the hint that a jekt may go to whichever instance registered last). A healthy row shows nothing.
2. **Row menu "Stop receiving messages"** (Deregister renamed for what it does), **only on a badged row**: it calls `AgentPaneUnregisterCommand` with the row's agent id and block id, the confirmation says the session keeps running and may register again on its next heartbeat. On a healthy row it does nothing lasting, so it is not offered there. Registrations and unregistrations are in the Audit (§4).
3. **Data.** One batched srv RPC, `reactive.registration_flags` (agreed with Camper 2026-10-29): it lists only the agents that have another claimant or a recent mismatch, each `{ agent_id, block_id, remote, recent_mismatch }` (the types `reactive.registrations` already returns), so the badge costs one call every 15 s while the Swarm is visible instead of a call per row. The Audit opens pre-filtered through pane meta `audit:agent` (the agent's id). `swarm-view.tsx` is 1,284 lines of 1,500: the badge and menu go in a new file, `swarm-row-registration.tsx`. Camper owns the Swarm; AgentY confirms the badge wording.

### LAN goes to the network pill (re-pinning stays in Settings)

The pill is the host popover (`statusbar/HostPopover.tsx`). It has the discovery toggle, the firewall and "can't be seen" explanations, the peer count and, per peer, hostname and version. What the LAN page had that the pill does not: address and port, the agent count, last seen. Peer rows become **expandable**: address and port, OS and channel (in `LanInstance`, never shown on the old page), the agent count with "Show in Swarm", last seen. The new component is `LanPeerRow.tsx` (`HostPopover.tsx` is 509 lines). The old page also had a bug of its own: it called `/api/lan-instances`, which its source says has returned 401 since it shipped. The pill reads the shared `lanInstancesAtom` instead.

**Correction to the earlier brief, which stands:** "re-pin / Accept the new key" is not in the pill; it is **Settings > Devices > Pinned senders** (Lazo's #5494, `pinned-senders.tsx`, `app_api/pins.rs`, `reactive/lan_refusals.rs`). The pill shows a warning on a peer whose key a message was refused for, and links there. Accepting a key is a trust decision that belongs in one screen beside its explanation.

**Internet** is removed. What it was meant for, the cloud relay, is the pill's "MuxBus Cloud" block.

Owners: the pill's recent authors are AgentX (connectivity, #5241, #5213), Clamk (#4495) and Lazo (typed LAN events, #4820; Lazo owns LAN on Area54). The build tells AgentX and Lazo first.

## 6. Ambient goes to the token pill

The token indicator (`statusbar/TokenUsageIndicator.tsx`) opens `TokenBreakdownPopover.tsx`, which already has a collapsed **"AgentMux internal" row** for ambient and background usage (`isAmbient`). That row is where Ambient goes:

- Expanding it shows what the Ambient section showed: one row per purpose (session titles, names, prompt suggestions, narration, running summaries, Supervisor verdicts) with Requests, Accepted, Kept, Refused, Skipped and Failed, from `AmbientOutcomesCommand` (unchanged), next to the tokens and cost those calls used. The popover is narrow, so each purpose is one line (name, failed count, a muted total) that expands to the six counts.
- The token indicator shows a small `am-error` dot when any purpose is unhealthy (what "Ambient · N failing" did in the pane title). The count is fetched every 60 s in the background and every 5 s while the popover is open, instead of Warden's 5-second poll that ran only while the section was shown.
- Drone's "AgentMux's own" group (§3) links its Ambient entries here.
- Deleted: `warden-ambient/`, `ambientFailing` and its title. Kept: `ambient-outcomes.ts` (the summary logic) and the RPC.

Owners: recent changes to the popover are chores and the shared `AnchoredPopover` (AgentA); the Ambient section was QTimer's (retired, #5317). Tell AgentX (status bar) and AgentY.

## 7. Redirects: every old Warden link keeps working

Today `MOVED_SECTIONS` (`pane-sections.ts`, mirrored in srv's `resolve_section` in `wconfig/types.rs`) maps one old section to another pane, with or without a section (since #5771 `to.section` is optional: `warden/kanban` and `swarm/kanban` both go to `{ view: "kanban" }`). Extend it to a table of **old pair → new target**, where the target is a pane (with a section), a status-bar popover, or nothing (with a notice):

| Old | Becomes |
|---|---|
| `warden` / `vault` (and `vault/<sub>`, `vault:section`) | `vault` (the sub-section carried over) |
| `warden` / `history` | `history` |
| `warden` / `audit` | `audit` |
| `warden` / `jobs` | `drone`, Jobs rail open |
| `warden` / `ambient` | the token popover, "AgentMux internal" row expanded |
| `warden` / `host` | the Swarm |
| `warden` / `lan` | the network pill popover |
| `warden` / `internet` | nothing: a notice "The Internet section is gone; the cloud relay is in the status bar's MuxBus Cloud block" |
| `warden` / `kanban` | the Kanban pane (already: #5771 points both `warden/kanban` and `swarm/kanban` at `view: kanban`) |
| `warden`, no section | **Audit** (it decrypts nothing, so a redirect the person did not ask for never prompts for the Vault key; Question Q1) |

The same table is read by everything that read the old one: the widget bridge (`am.panes.open("warden", { "warden:section": "lan" })` works and says where it went), `sectionProblem` (an old pair is not an error in Settings > Widgets), the widget bar's pins, the palette, and srv's `resolve_section` for `widgets.json`. A popover target needs the status bar's popovers to be openable from code; `StatusBar.tsx` builds them from a list, and whether that list can be driven externally is the first thing the build checks. If not, the fallback is the notice alone.

**Everything that names Warden, and what it becomes:**

| Where | Today | After |
|---|---|---|
| **Saved panes** (block meta `view: "warden"`) | opens on its saved section | a one-time srv startup migration rewrites each such block to its new view per the table (like the History block migration in §8); a Warden pane saved on Host becomes a Swarm, on LAN a notice. No Warden code remains to load it |
| **Layout files** (`layout_file.rs`: `type: "warden"` with `config.section`, and a `vault` pane written as Warden's Vault, lines about 507, 605 and 1151, with tests in `layout_file/tests.rs`) | reads and writes Warden | reads old files through the table; writes `vault`, `history`, `audit`, `drone` as their own panes; tests updated |
| **Widget bar** pins by key | a saved bar naming `warden` | a dangling pin is replaced by `vault`, `history` and `audit` in its place (Question Q1) |
| `defwidget@warden`, `defwidget@vault` | Warden, Vault | Warden deleted; Vault retargeted to `view: vault`; History and Audit added |
| **Command palette** (`command-registry.ts`) | `open:vault` (Warden's Vault) | `open:vault` opens the Vault; `open:history`, `open:audit` added; `open:jobs` added (Drone with the Jobs rail). `open:kanban` already opens the Kanban pane |
| **Widget SDK** (`sdk/widget-sdk/v1.d.ts`, `PaneSections`; `srv-types.d.ts`) | `warden` with eight section ids | the type no longer offers `warden`; old calls resolve through the table. Mark it in the SDK changelog |
| **Agent MCP routes** | none open a Warden section | none change. Only text: the Vault tool schema (`crates/mcp/src/tool_schemas.rs`, "a section of the Warden pane"), and `window_capture.rs:585` ("until the person leaves its Warden section") say "the Vault pane" |
| **Incident text** (`backend/jobs/incidents.rs`:633, 645, 657) | "Warden > History", "Warden > Jobs" in Kanban tasks and leader notes | "History", "Drone > Jobs"; the tests (`incidents_tests.rs:94`) updated. Already-filed tasks keep their old words (prose, not a link) |
| **`README.md`** rows 67 to 69, 146 and 162 | Warden with the Kanban, Host/LAN/Internet | rewritten: three widgets, Drone's Jobs, the Kanban pane; the "Pin LAN to bar" example uses a section that exists |
| **`widgets.json` descriptions** | "Monitor and control AgentMux instances across Host / LAN / Internet layers..." | gone with the entry; new descriptions for the three |
| **docs.agentmux.ai** `/warden/` and `/lan-discovery/` | not in this clone (the `agentmux-docs` repo) | **A task for the docs owner (AgentY's lane), filed by the build:** the Warden page splits into Vault, History, Audit and Drone Jobs pages; `/warden/` stays as a page that says where each section went, so old links resolve; `/lan-discovery/` says peers are in the network pill. Not edited from this build |
| **Old specs** that say Warden | e.g. `SPEC_WARDEN_WIDGET_2026-05-25.md` | stay as records; this spec supersedes the three-layer model in it |

## 8. The History bug and its fix (unchanged by the new direction)

**What happens.** In History, "Open history" calls `openOrFocusHistoryTab` (`agent/open-history-tab.ts`), which stacks into the **current pane** a new block of view `"agent"` carrying `agentId` and `agent:historyTabFor`. The frontend renders that block as a read-only reader, because `AgentBlockContent` checks `agent:historyTabFor` first. But a block with `view: "agent"` and `agentId` is also exactly what a live agent pane looks like to the server, and several server paths identify "the agent's pane" by those two keys with no knowledge of the history key: `app_api/agent_block.rs` (`is_agent_block_of`), `sagas/orphan_reaper.rs` (`is_agent_view`), `sagas/park_agents.rs` (`is_parkable`), `backend/agent_seat.rs`, `viewer/panes.rs`, and `agent_open.rs`'s `find_agent_block`. `crates/` never mentions `agent:historyTabFor`. So opening a history tab makes the agent appear to have a second pane. Which of those starts a live session in the reported case I have not reproduced (Manoz is tracking it).

**What "Open history" should do.** Open a **read-only reader** of the agent's recorded conversation. It never launches, resumes, registers or counts as the agent's pane, whether the agent is running, stopped, parked or deleted. For a running agent the live pane stays where it is; a "Go to its live pane" link in the reader's header is a separate, explicit click. Opening twice focuses the first.

**How.**
- **A. The proper fix.** A reader that is not an `agent` block: a pane tab with its own view (`agent-history`) and meta `{ "history:agent": <definition id>, "history:name", "history:provider", "history:sourceBlock"? }`, no `agentId`, rendering the existing `AgentHistoryView` unchanged. `openOrFocusHistoryTab` keeps its signature and its callers (the History pane, the agent picker, the live pane's history link and menu) and opens that view. The Audit's "In history" links use it, with an extra `at_byte` or `at_ms` the reader scrolls to.
- **B. Stopgap and migration.** Two lines in each of the six server paths: "a block with `agent:historyTabFor` is not an agent pane"; and a one-time srv migration converting existing history blocks to the new view. The guard can ship first, since the bug is live.

## 9. Build plan (#9205)

Five phases. Phase 1 stands alone and fixes the live bug; 2 and 3 are the removal; 4 and 5 are the larger additions and can follow.

1. **History reader (§8)**, B first, then A. Not blocked on anything else. Owners: Agent5, Manoz.
2. **Split.** The three panes (Vault, History, Audit as it is today), `defwidget` entries with `singleton`, the redirect table in both languages, the srv block migration and layout-file changes, the palette and SDK changes, incident and tool text, README. Warden's code is deleted. Host's badge and menu (§5), the pill's peer detail (§5), the token-pill Ambient (§6). Depends on #9110 (the singleton flag) for the flag only: the entries can ship with the field ignored.
3. **Jobs in Drone.** The job rail, derived diagrams for cron and Drone and cloud jobs, the moved components, title and failing count. Owner of record to be named by AgentX.
4. **Audit, beefed up.** `audit.query`, reading the durable file with filters; start and end from the segment ledger; the jekt links both ways; Swarm's "Audit for this agent". Then the event index and tool-call and thought entries, then the live pane's scroll-to-position.
5. **AgentMux's own scheduled work** in Drone (§3.5).

Each phase is a PR with a changeset; none touches versions. The checks on a task dev build, from the builder's own checkout: the three panes open and reopen as singletons, a pin or saved pane naming each old section lands on its new home, a job of each type shows its diagram, a jekt row's links open the sender's and receiver's history at the right message and a closed pane still works.

## 10. Questions for the operator

Only what is still open. Each has a recommendation.

- **Q1. Where do old references to the whole Warden go?** A saved Warden pane with no section (and a `warden` pin on the widget bar) must land somewhere. *Recommend:* the pane opens as **Audit** (read-only, decrypts nothing); a pinned Warden is replaced by Vault, History and Audit side by side so the person loses no way in. Alternative: Vault, which is what most people opened Warden for, at the price of the OS key prompt on a redirect they did not ask for.
- **Q2. Are Vault, History and Audit pinned on the widget bar by default?** *Recommend: no*, as Warden and Vault were not: they are occasional, they live in the More menu, and Vault's entry already worked that way.
- **Q3. Is Drone's job list always visible, or a mode?** *Recommend:* a collapsible left rail that is open when Drone opens from "Jobs" and the palette's `open:jobs` and remembers its state otherwise. The alternative, a toggle between "Graph" and "Jobs", hides the diagram you came for.
- **Q4. Derived diagrams are read-only until Drone has triggers.** Is that acceptable for now? *Recommend yes*: a cron job is edited in its form and the diagram redraws. Making a cron job an editable Drone graph needs Drone to start from a trigger (Jobs P4), which is a different spec.
- **Q5. Tool calls and thoughts: record only from the build forward, or backfill?** *Recommend:* from the build forward, plus a lazy per-agent backfill the first time you filter to that agent (bounded scan, shown as "indexing"). A full backfill of every transcript at once is a large, silent read of the history store.
- **Q6. Providers without tool-call and thought entries.** The index starts with Claude, whose stream the adapter classifies; other providers get start, end, jekt and action entries and a visible "not recorded for this provider". *Recommend yes*, with the adapters filled in as each provider's format is understood.
- **Q7. Is "scroll the live pane to that position" worth building,** or is the history reader at the position enough? *Recommend build it, in phase 4*, since "open the exact agent pane" was the ask; the reader covers every case until then.
- **Q8. AgentMux's own scheduled work in Drone (phase 5): yes or later?** *Recommend yes, read-only and last*: it adds data rather than moving it, and nothing else waits on it.
