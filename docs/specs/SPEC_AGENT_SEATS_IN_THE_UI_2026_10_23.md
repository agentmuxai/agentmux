# SPEC: Agent seats in the UI: role, lead and area on the agent, shown where you meet it

**Date:** 2026-10-23
**Status:** active — approved by the operator 2026-10-23 (open questions as proposed: operator-only seats in v1, the seat in startup instructions in phase 2, seats inform and don't gate). PR 1 (data and API) in #5378; PR 2 (the Seat tab, roster import, My Agents tile and header tooltip) in #5382; PR 3 (Swarm badge and Group by lead, Kanban owner tooltip and "Reports to" filter) in #5405; other instances' seats in the Swarm (§5) built after PR 3; PR 4 (docs) not started.
**Trigger:** The operator, 2026-10-23: *"how does the user know what seat AgentX is in?"* Seats were approved that day: which agent leads, which reports to whom, and what each owns. Today a seat exists only in a Global Memory entry ("Agent seats: …") and an internal report. Nothing in the app shows it.
**Related:** the internal organization spec's phase 4 (a lane and a role on the agent definition, Swarm grouped by lane; not public, so only its conclusions are used here); `crates/srv/src/server/app_api/agent_spawn.rs` (`agent:worker_of`, the spawned-worker relationship); `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` (identity is the uid, names are labels).

## 0. Summary

- A **seat** is three facts about an agent: its **role** (leader, worker, specialist, platform tester, analyst, with an optional qualifier such as "product"), its **lead** (the agent it reports to, by uid, or "the operator"), and its **area** (one line: what it owns). An optional **lane** is kept for the organization spec's later phases.
- The seat lives on the agent's **shared definition** (`DefinitionRecordV1`), so every channel and version on the machine sees the same seat. It's an additive field: no schema bump, no `db_agents` column.
- **The operator sets a seat** in the agent's Stash drawer (a new Seat tab) or with an import of the approved roster. Since #4477 an agent can also set or clear seats on its machine with `Agents set_seat`, recorded under its own name (the operator's decision, 2026-10-25: agents are first class).
- **Where you see it:**
  - the My Agents tile (a role badge, plus "reports to … · area");
  - the agent pane header's tooltip;
  - Swarm (a badge on each row, and **Group by lead**);
  - the Kanban card (the owner's seat on hover);
  - MuxAPI's `Agents list`, so leaders read seats from the app, not from a memory entry.
- **"Reports to" has one meaning.** A spawned worker's `agent:worker_of` already says who leads it. A worker with no seat of its own shows that as its lead (role: worker, "spawned"). A seat set by the operator wins.

## 1. Today

- `AgentDefinition` (`crates/srv/src/backend/storage/agents/mod.rs`) has `name`, `description`, `parent_id`, `branch_label` and no role or lead. Only `AgentCard`'s caption shows `description`, and only for templates.
- A fork's `parent_id`/`branch_label` say where it came from, not whom it reports to.
- Spawned workers carry `agent:worker_of` (agent content, written only by srv) and `agent:coached_by` (block meta). That's the one place "reports to" exists in the code.
- Agent groups (`db_agent_groups`) are saved fleet-control target sets of block ids, per channel. They aren't a team model, so this spec doesn't reuse them.

## 2. Data

```
seat: {
  role: "leader" | "worker" | "specialist" | "platform_tester" | "analyst",
  qualifier?: string,        // e.g. "product", "platform", "macOS" — shown after the role
  lead?: string,             // the lead's agent uid; absent = reports to the operator
  area?: string,             // one line, at most 120 characters
  lane?: string,             // reserved for the organization spec's lanes; not shown in v1
  set_at_ms: number,         // when the operator last set it
}
```

**Where:** `DefinitionRecordV1.seat` (`crates/srv/src/registry/def_schema.rs`), `#[serde(default, skip_serializing_if = "Option::is_none")]`. It's additive, the same way `use_ambient_login` and `former_slugs` were added: an older binary keeps it through `merge_for_write`.
- `AgentDefinition` gains `seat: Option<AgentSeat>`, one nested field. That keeps the ~100 struct-literal sites to one added line each. `#[serde(default)]` applies, and the ts-rs export follows.
- `def_registry_mirror.rs` maps it both ways, and `agent_def_list` copies it back from the record when it lays the record over the local row (`definition.rs`, as for `memory_id`). Otherwise the overlay would drop it on every read.
- No `db_agents` column: the seat is the shared record's, like the agent's name. A channel-local agent with no shared record has no seat. That's rare today, and the Seat tab says so for it.

**Validation (srv):**
- `role` must be one of the five.
- `lead`, when present, is an existing agent's uid on this machine, or a uid the LAN directory knows (§5), and not the agent itself. As built in PR 1, only this machine's agents are accepted: a LAN peer lists its agents by name, not by uid, so there is no uid to check a remote lead against yet.
- No cycles: following `lead` must reach the operator.
- `area` is trimmed and at most 120 characters, and `qualifier` at most 40.
- A leader's `lead` is normally absent (the operator), but that isn't enforced.

**Spawned workers:**
- A worker with `agent:worker_of` and no seat is shown with an *effective* seat: role worker, qualifier "spawned", lead = its leader. It's computed when the definition is read, never stored.
- When the operator sets a seat on such a worker, the stored seat wins.

## 3. Who writes it

- **The operator, and (since #4477) agents.**
  - **Writes from the window go through the RPC `agent.setSeat {agent_id, seat | null}`.** It's allowed only for the operator's own client (the app's renderer), refused for any `Caller::Agent` (MuxAPI, MCP, jekt), and audited. An agent uses `Agents set_seat` (`POST /api/v1/agent/seats`) instead, with the same validation, audited under the calling agent.
  - **`agent.define` and `updateagent` don't touch `seat`.** A caller that omits it keeps the stored value, using the "None = keep" pattern `model_vendor_base_url` uses, so an identity or rename save never wipes a seat.
- **The editor:** a **Seat** tab in the agent's Stash drawer (`AgentStashModal.tsx`, data-driven tabs). It has the role, a qualifier, the lead (a picker of agents, plus "the operator"), the area, Save and Clear. My Agents' row menu also gets a **Seat…** item that opens it. As built in PR 2, that item opens the same panel in a dialog over the picker, because the Stash drawer lives in the agent's own pane, which may not be open.
- **The initial roster:** the approved roster arrives as a small JSON (agent name or uid → role, qualifier, lead, area). The operator imports it from the Seat tab's "Import roster…". It's resolved by uid or exact name, previewed, then applied. Unresolved names are listed, not guessed.
- **Agents read seats** (`Agents list`, §4). Since #4477 they may also write them with `Agents set_seat`.

## 4. Where it shows

| Surface | What | Where in the code |
|---|---|---|
| **My Agents tile** | A role badge after the runtime badge ("Leader · product", "Specialist"), and a line under the account line: "reports to Camper · panes, layout, tabs" | `MyAgentsList.tsx` line 1 and after the account line; `RecentSessionRow` gains `seat` (`agent_handlers/session.rs`) |
| **Agent pane header** | The tab's hover tooltip adds "Leader · product — reports to the operator — agent pane, identity, …" | `PaneHeaderTabStrip.tsx` `summaryOf`; the seat is seeded into block meta (`agent:seat`) at `agent.open`, as `ui:color` is, and refreshed when the seat changes |
| **Swarm** | A small role badge after the name. The view gets a **Group by: none / lead** control: by lead, each leader heads its group and the operator heads the leaders | `swarm-view.tsx` row; `AgentTreeNode` gains `seat` from block meta |
| **Kanban** | A card's owner shows the owner's seat on hover; a filter "reports to …" | Warden Kanban card owner chip |
| **MuxAPI `Agents list`** | Each `host.agents` entry gains `seat: {role, qualifier, lead: {id, name}, area}`, from its definition | `handle_discovery` (`http_health.rs`), by `definition_id`; the `Agents` tool description says so |

As built in PR 3:
- Swarm reads `agent:seat` from each row's pane meta where it draws the row, not through `AgentTreeNode`: `swarm-model.ts` is at its size limit. The choice is kept in the Swarm pane's meta (`swarm:groupby`), as its zoom is. A lead with no pane open heads its own group by name. Other instances' rows were put under their own heading at first, because `swarm.other-instances` carried no seats. It now does, for LAN peers (`RemoteAgent.seat`, from the peer's `agent_seats`): a row shows its role badge, and is grouped by its lead's name within its own host and channel; a remote lead the peer couldn't name heads a group "an agent on another machine". A remote agent with no seat, and this machine's other channels (which report none), stay under "Other instances".
- The Kanban reads seats from `listagents`, not from the board. The filter keeps the tasks whose owner (the agent holding the task, else the one it waits for) reports to the chosen lead. That lead's own tasks are not included.

The agent's own instructions are left alone in v1. The roster Global Memory entry remains for agents launched before a seat shows in `Agents list`. Phase 2 can compose a one-line "Your seat: …" into the agent's startup file, from the stored seat.

## 5. Agents on other machines

- **Each machine stores the seats of its own agents** (its shared definitions). A seat for an agent on another host is set on that host, by the operator there.
- **The LAN directory carries seats.** The per-peer agents list a LAN peer serves (already fetched for the LAN section) gains each agent's `seat`. Swarm's remote rows and `Agents list`'s `lan` entries show it. A lead may be a remote agent: stored by uid, it shows by name when the directory knows it, and as "an agent on another machine" when it doesn't.
- WAN (cloud) agents show no seat in v1. The relay has no directory.

## 6. Relation to the organization spec

- **Its phase 4** puts a lane and a role on the definition, groups Swarm by lane, routes the Kanban by lane and names spawned followers by lane. This spec builds the role, the lead and the area, plus a reserved `lane` field, which phase 4 can fill and use.
- **Grouping by lane and routing tasks by lane** are left to that phase.
- **Seats are labels on an unchanged identity** (uid and slug), so a seat change never renames anything and never moves memory or keys.

## 7. Tests

- **srv:**
  - the seat round-trips through the shared record and through `agent_def_list`'s overlay;
  - an older-schema record keeps it;
  - `updateagent` and `agent.define` without a seat keep it;
  - `agent.setSeat` refuses an agent caller and accepts the operator's client;
  - validation: unknown role, a lead that is self, a missing lead, a cycle;
  - a spawned worker's effective seat;
  - `Agents list` carries seats;
  - the LAN agents list carries seats.
- **Frontend:**
  - the Seat tab (edit, clear, import preview with unresolved names);
  - the My Agents tile badge and line;
  - the header tooltip;
  - Swarm's badge and Group by lead (leaders under the operator, workers under leaders, unseated agents under "No seat");
  - the Kanban owner tooltip.
- **Live:** an isolated `task dev` with `AGENTMUX_HOME_OVERRIDE`. Import a three-agent roster, check each surface, change a lead, check Swarm regroups.

## 8. Rollout

| PR | Scope |
|---|---|
| 1 | Data, validation, `agent.setSeat`, the effective seat for spawned workers, `Agents list` and the LAN list carry seats |
| 2 | The Seat tab, My Agents row menu item and roster import; My Agents tile; header tooltip |
| 3 | Swarm badge and Group by lead; Kanban owner tooltip and filter |
| 4 | Docs: a "Seats" section on the agents page, and glossary entries for seat, lead and area |

## 9. Open questions for the operator

1. **Should leaders be able to set their own reports' seats?** Proposed: no in v1, operator only.
2. **Should an agent's startup instructions say its seat** ("You are the product leader; you report to the operator; you own …")? Proposed: phase 2, after the UI.
3. **Should a seat be enforced anywhere** (for example, the Kanban refusing a task outside the owner's area)? Proposed: no. Seats inform, they don't gate.
