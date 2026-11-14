# Spec: Atlas, a map of every agent across machines and tiers

**Status:** active. Approved by the operator on 2026-11-13; the build (#47548) ships v1 as this spec's section 6 says.

Task #47547 (build: #47548, after this). Owner: Camper (panes, layout, Swarm). Written 2026-11-13 from origin/main
32d862d3d.

## 1. What the operator asked for

In their words (2026-11-13): "we want to be able to see a graph of our systems. ... it is a map of all the agents across
the tiers with data about what they are doing. essentially a geographic look at things (as opposed to the swarm's list
based)". The name is decided: "atlas is good".

Fixed by those words: a pane; a graph laid out in space; every agent, across the tiers; each agent shows what it is doing;
the spatial counterpart of the Swarm's list. Everything else below is a recommendation to decide.

## 2. What the app already has (the survey)

Read from the code, with file paths in the notes at the end. The short version: **most of what a map needs is already in
the frontend for the Swarm; two things are missing (message flow, PR/branch) and three are cheap to add.**

| Need | Exists today | Where |
|---|---|---|
| Every local agent, state, status line, current tool, context size, spend today, last failure, seat | yes | `AgentTreeNode` (`swarm-model.ts`), block meta `agent:*` |
| Agents on other machines (LAN, and this machine's other channels) | yes, thinner: name, status, line, provider, seat | `RemoteAgent` (`swarm-remote.ts`), srv `swarm_remote.rs` |
| Machines | yes | `HostInfo`, `LanInstance` (hostname, os, version, channel, last seen) |
| Seat, area, and who it reports to | yes | `AgentSeat` (`lead`, remote leads as `Name@host`), meta `agent:seat` |
| Spawned workers and subagents | yes | `agent:worker_of`; `ActiveSubagent.parent_agent` |
| Runtime (host or container) | in srv, not shown | `RuntimeKind` |
| The relay's health | yes, status bar only | `ConnectivityStatus` |
| An agent's repo, branch, goal, recent files | in srv, not read by the frontend | `work_facts` (`Agents conversations`) |
| Account per remote agent | srv sends it, the frontend drops it | `agent_accounts` |
| Messages between agents, task hand-overs | **no**: no list, no history | gap |
| The review bot, CI, containers as systems | **no** | gap |
| A "cloud" tier | typed in the frontend, srv never sends it | gap |
| Graph layout or pan/zoom to reuse | pan and zoom yes (the Drone canvas); auto-layout no; no graph library in `package.json` | `drone-view.tsx` |

Scale: about 70 agents on one machine, most stopped. The Swarm folds stopped agents into one "Stopped agents" section; every
row is in the DOM (no virtualisation), which is fine at this size.

## 3. Proposal in one paragraph

Atlas is a pane, available from the "+" menu like any widget and to every user (its data is what the Swarm already
shows; a user with no seats just gets machines and agents, no lines). It draws **machines as regions and agents as nodes
inside them**, with **lines for "reports to"** (and dashed lines for "spawned"), so a leader on one machine and its
workers on another read as one picture. A node shows state at a glance; hover or click gives the card. It is **read-only
apart from the Swarm row's own actions** (focus or open the agent's pane, Start a stopped one). A second layout, **By
seat**, redraws the same nodes as an org chart; the machine becomes a colour on the node. Messages in flight and task
hand-overs are the first thing added after v1, because they need a new srv event.

## 4. Layout sketches

### 4.1 By machine (the default)

```
 Atlas                                   [By machine | By seat]   [fit] [+] [-]   filter: ____
 ┌───────────────────────────────────────────────┐   ┌─────────────────────────┐
 │ desk  (this computer)  0.60.26  stable       │   │ lab-1     Linux  0.60.26│
 │                                               │   │                         │
 │        (Lead-A)●────┐          (Lead-B)●────┐  │   │      (Eve)●           │
 │          │          │             │         │  │   │        │                │
 │       (Ann)◐    (Bo)●        (Cy)●     (Di)◐│   │  (Fay)●   (Gus)○  │
 │          :                                    │   │                         │
 │      (Ann-sub)·                            │   │  3 stopped ▸            │
 │                                               │   └─────────────────────────┘
 │   52 stopped ▸                                │   ┌─────────────────────────┐
 └───────────────────────────────────────────────┘   │ lab-2   macOS  0.60.26│
        (Lead-B ──────────── reports to ─────────────▶ (cross-machine lines     │
                                                      │  cross the gap)         │
                                                      └─────────────────────────┘
  ● working   ◐ waiting / blocked   ○ idle   ✕ error or signed out   · subagent
```

- A region per machine (this computer first, then LAN peers, then this computer's other channels).
- Inside a region, leaders are placed on top and the agents that report to them below; agents with no lead sit in a row at
  the bottom. A solid line is "reports to"; a line that leaves a region is a cross-machine lead (`Lead@host`).
- Stopped agents are not nodes: each region has a "N stopped ▸" chip that unfolds them into a compact list.

### 4.2 By seat

```
                        (Operator)
                 ┌─────────┴─────────┐
              (Lead-A)            (Lead-B)
        product leader       platform leader
         ┌───┬───┐            ┌──────┬───────┬──────┐
        Ann  Bo  Cy        Di     Eve    Fay    Gus ...
         d    d   d          d      l1     l1     l2        d = desk, l1 = lab-1, l2 = lab-2
```

The same nodes as an org chart by lead. The machine is a small badge and a tint. This is the view for "who is doing
what for whom"; By machine is the view for "what is each computer doing".

### 4.3 A node and its card

At rest: provider icon, name, a state dot, a seat badge, and a small ring when the context is nearly full. On hover or
click, a card: status line (the Swarm's own line), the current tool, context size, spend today, the held Kanban task
(local agents only), the last failure or sign-in state, the machine and runtime (host or container), and the Swarm's
actions (focus or open, Start).

## 5. Decisions for the operator

Each has a recommendation; "yes" to all of them is a complete answer.

1. **What are the tiers?** Both: machines and seats, as two layouts of the same data. *Recommend:* build **By machine
   first** and By seat in the same PR if it stays small (it reuses the nodes and the lines); the toggle is saved in the
   pane's meta. *Alternative:* By machine only for v1.
2. **What does a node show?** State at rest; the card on hover or click (section 4.3). PR and branch are **not** in v1
   (the frontend has no source; see 5.5). *Recommend:* as written.
3. **What are the lines?** v1: **reports to** (solid) and **spawned** (dashed). **Messages in flight** (animated pulses)
   and **task hand-overs** are v2: no message or hand-over history exists today, so v2 needs an srv event that says
   "A sent B a message at T" (ids only, never text). *Recommend:* v1 as written; file the srv event now so v2 follows.
4. **Do non-agent systems appear?** v1: **machines** (the regions) and **MuxBus** as one badge on the title bar, from
   `ConnectivityStatus` (the status bar already has this). The review bot, CI and containers as their own nodes are v2:
   they have no status in the frontend. *Recommend:* as written.
5. **Where does the data come from?** The Swarm's own store (`AgentTreeNode` and `RemoteAgent`); no second data path.
   Three small additions, all in the frontend: show the account that srv already sends for remote agents, read
   `work_facts` for PR and branch on the card (after v1), and show runtime kind. Nothing new in srv for v1. *Recommend:*
   as written.
6. **What can you do from the map?** Read-only plus the Swarm row's own actions (focus or open the pane, Start a stopped
   agent). **No message, stop or broadcast from the map in v1**: the Swarm has no single-agent stop or message either, and
   the fleet toolbar acts on a selection, which a map could add later (lasso, then the same Broadcast and Bulk stop).
   *Recommend:* as written.
7. **Scale and stopped agents.** Stopped agents fold into a per-machine chip (matches the Swarm). Layout is computed
   in the pane by our own small deterministic layout (regions in a row or grid, leaders above reports, no physics), so
   it does not move on every refresh. No graph library is added: `package.json` has none, the Drone canvas shows
   hand-rolled SVG with pan and zoom is enough, and 70-ish nodes need no virtualisation. *Recommend:* as written.

## 6. Plan (the build, #47548)

One PR, in this order, each behind the pane existing:

1. The `atlas` view: registration, "+" menu entry, pane header, empty states (no agents, no peers).
2. The data adapter: one function from the Swarm's tree and remote rows to `{regions, nodes, edges}` (pure, unit tested,
   including remote leads written `Name@host` and agents whose lead is missing or unknown).
3. The layout: deterministic, pure, tested (stable positions across refreshes, no overlap, 0, 1 and 70 agents).
4. The SVG render and pan, zoom and fit (the Drone canvas's anchored wheel zoom pattern), keyboard focus on nodes.
5. The card, the filter (the Swarm's `name:` and `work:` prefixes), and the layout toggle.
6. Look checks on macOS and Linux (visual), light and dark, the narrow-pane rule (works down to 280 px: the card becomes a
   sheet, the map scrolls and zooms).

Out of v1, filed as follow-ups: the message-flow event and pulses; PR and branch on the card; review bot, CI and
container nodes; lasso select to Broadcast or Bulk stop; the cloud tier (typed, never sent).

### 6.1 After v1: selecting on the map (#47554)

Built as decision 6 foresaw. The map selects into **the Swarm's own selection** (the pane's `SwarmViewModel`), and the
actions are the Swarm's own components, so nothing about reach, approval or reporting is decided twice:

- **Selecting.** A drag on the background still pans. With **Shift** held it draws a box, and the agents the box
  touches join the selection when it is let go (Escape drops it). **Select**, a toggle in the bar, swaps the two for
  a trackpad or a touch screen: a drag selects and Shift pans. A Shift, Ctrl or Cmd click on a node (any click with
  Select on) adds or removes that one agent. A box only adds; "Select none" clears.
- **What can be selected** is what the Swarm can tick: this instance's agents that have a pane, and the agents of
  other instances whose machine is not stale. A stopped agent is not a node and is not selected from the map.
- **Acting.** While anything is selected (or Select is on), a strip under the map holds the Swarm's Select all and
  count, **Stop** (with the Swarm's confirmation, which lists every agent and what it cannot reach), the Supervisor
  **Mode** menu, and the **Broadcast** composer; the per-agent result panel follows. It is under the map, not over
  it, so the map's top left stays where it is when the strip appears. An agent on a LAN peer gets a broadcast and
  cannot be stopped from here, exactly as in the Swarm.

### 6.2 After v1: messages and task hand-overs on the map (#47554)

Decision 3's second half, on the two srv events that were filed for it: `message-flow` (#7173) and `task-flow` (#7192).
Both carry ids, hosts and times only, so the map shows that something passed between two agents and never what.

- **A message** draws a line from its sender's node to its receiver's as srv handles it: the line draws itself in that
  direction, lands on a dot, stays about 12 seconds and fades. Its colour says what became of it: the accent for
  delivered, muted for held or queued for later, the warning colour for refused. A repeat between the same two
  agents draws the line again; a message and its answer bow to opposite sides, so both show.
- **One end off the map** (the other agent is stopped, filtered out, or not an agent of a machine listed, such as
  the review bot until it has a node): a ring round the end that is, which fades.
- **A task one agent hands another** (filed for, moved to or started on it) is a dotted line with the task's
  number, for five minutes; at most twelve show, the oldest going first. A task the operator filed, or one for
  anyone, has one end and draws nothing.
- **When the pane opens** it reads srv's recent lists (`GET /api/v1/message-flow`, `/api/v1/task-flow`), so the last
  seconds of messages and the last minutes of hand-overs are already there.
- **Which node an end is.** srv names an end by the name messages reach the agent by and, when it knows it, its
  host. This instance's agents carry that name in their pane (it differs from the display name for a renamed
  agent); another instance's agent is matched by its name on its machine. An end that matches no node, or two, is
  not drawn: no line rather than a wrong one.
- **Lines run behind the nodes.** A line between two agents that are not neighbours passes behind the nodes in
  between, so it never crosses a name; its ends show at the two nodes' edges. A task's number sits on the stretch
  of its line nearest the middle that no node covers, in the text colour (the look checks on macOS and Linux both
  found a number landing on another node's name, and dim on the dark theme, with the lines in front). Agents side
  by side in a row leave no such stretch (the gap between two is narrower than a number, and the line is behind
  the ones in between), so there the number goes just under or just over the node the line lands on, at the corner
  it lands on (the Linux re-look found the number hidden behind the node in between). A number also keeps clear of
  the numbers placed before it. Two tasks handed to one agent in a row land on the same corner, so the second
  number takes the other side of the node, and a third goes a number's width along the node's edge (the same
  re-look found one number drawn on top of the other). A task and one handed back between two agents close
  together bow apart by less than a number's width, so the second number goes next to the first.
- **Motion.** With the Animations setting on Off, a line appears whole and goes after its time; nothing travels.
  The operating system's own "reduce motion" preference alone does not change it, as everywhere in the app
  (the motion policy's 2026-10-15 decision).

### 6.3 After v1: more on the card (#47553)

Decision 5's three additions. Each is a row on the card that shows only when its fact is known; the node at rest
does not change.

- **Runtime.** Host or container, with the badge the agent pane uses, for this instance's agents (from the pane's
  own meta). Another instance's agents say nothing about it yet; that comes with the container status work
  (#47595).
- **Account**, for an agent of another machine. Its machine reports the account as an id every machine agrees on
  (a provider and a hash, never an email). srv now adds this computer's own label for it when the same account is
  signed in here (`swarm.other-instances`, `account_label`), so the row reads "Work"; otherwise it reads "Not
  signed in on this computer". Under it, the other agents on the map that run on the same account, since they
  use up the same quota. Nothing about an account leaves the machine that did not already.
- **Branch.** The repository and branch of the agent's clone, from srv's work facts (`GET /api/v1/work-facts`),
  else the branch its newest held task was filed with.
- **Tasks.** The Kanban tasks it holds (claimed, waiting on a question, or in review), the three touched last,
  each with its pull request as a button that opens it. A title is cut after two lines, and the whole of it is
  its hover label (the Linux look check had one title fill four lines of the card).

Branch and tasks are this instance's facts, so they show for its own agents only. They are read while a card is
open and not otherwise: the work facts once and then every 20 seconds, the board through the Kanban's own model.

### 6.4 After v1: containers, the review bot, CI and the cloud tier (#47607)

Decision 4's second half and the cloud tier, on the data three srv builds added: this machine's agent containers
(`SPEC_CONTAINER_STATUS_IN_THE_FRONTEND_2026_11_14.md`), the review, checks and merge events
(`SPEC_REVIEW_AND_CHECKS_FLOW_EVENTS_2026_11_14.md`) and the account's other machines
(`SPEC_SWARM_CLOUD_TIER_2026_11_14.md`). All three carry names, numbers, states and times only.

- **A container agent** has a dashed frame round its node and a box after its name, on this machine and on any
  other that says where its agents run. For this machine's agents the frame is in the colour of the container's
  state (muted while it runs, the accent while it starts, the warning colour when it has stopped under a running
  agent), and the card names the container, its state and since when, and its image.
- **A container no agent uses** (srv lists it as an orphan) is a node of its own in this machine's region, dashed,
  after the agents: its card says its state and image. It is not an agent, so no box selects it, and with a filter
  on it is not drawn.
- **The container runtime not running** is said after this machine's badge, when the machine uses containers at
  all (an agent of it runs in one, or one is listed).
- **The review bot and CI** are two pill-shaped nodes in a region of their own, "Pull requests", after the
  machines; in the By seat layout they stand with the agents that have no lead. Neither is drawn until a notice has
  come, so a machine with no review bot has a map of agents only. Each node's second line is what needs attention
  ("2 need changes", "1 failing"), else when the last notice came, else that cloud messages are paused (MuxBus needs
  a sign-in, or has been out of reach for longer than the status bar stays quiet about). Its card lists the pull
  requests it has a line for, newest first: the number, what became of it, whose it is, and when.
- **A review, and a pull request's checks,** draw a dashed line from the node to the agent whose pull request it
  is, with the number ("#64"; "#64 Codex" or "#64 person" when the bot was not the reviewer): green for approved or
  passing, the warning colour for changes requested or failing, muted for anything else, a state this build does
  not know included. The newest review and the newest checks of a pull request replace the ones before. A line is
  bright for five minutes and then stays dim for as long as the pull request is open: its merge takes its lines
  away, and one this machine never hears the end of goes after two hours. At most twelve of each kind show. The
  agent is always one of this instance's own (srv reports the notices it delivered), so a line never lands on an
  agent of another machine that has the same name; with that agent stopped or filtered out, no line is drawn.
- **The cloud tier** needed no new drawing: srv now sends the account's other machines as hosts of the Swarm's
  third tier, so they are regions after the LAN's, with the badge "cloud" and a cloud mark, and no surface of their
  own (an outline). Their rows are thin: a state and a line, no seat, provider or account. They can be ticked as in
  the Swarm, whose confirmation says what a broadcast or a stop cannot reach. When srv's last read of them failed,
  the map says so in its bottom left corner.

## 7. Risks and notes

- **"Steward" is not a stored seat role** (`ROLES` is leader, worker, specialist, platform_tester, analyst, super), so a
  steward draws as a worker until a seat role is added. Not an Atlas problem; worth knowing when the By seat layout shows.
- **Remote rows are thinner** than local ones and go stale after 60 s (hidden after 10 min): a stale node is drawn muted
  with its age, the way the Swarm does.
- **Motion:** nodes do not glide on refresh unless animations are on; the pane uses the existing motion tokens and the
  reduced-motion setting.
- **Container agents and every user:** the pane only reads what the Swarm already reads, so it works for container agents
  and for users with no seats, no peers and one machine (one region, no lines).
- **The survey came from a read-only pass over the code** (file references below); the first thing the build does is
  re-read each of those so the adapter is written against the real types.

## 8. Where each fact lives (for the build)

- Local card data: `frontend/app/view/swarm/swarm-model.ts` (`AgentTreeNode`, `buildTree`), `swarm-agent-card.tsx`,
  `swarm-display-status.ts`, `frontend/app/store/swarm-line.ts`, meta `agent:context`, `agent:seat`, `agent:spend_today`,
  `agent:last_failure`.
- Remote: `swarm-remote.ts` (`RemoteAgent`), `crates/srv/src/backend/swarm_remote.rs`, `frontend/types/rpc/LanInstance.ts`.
- Seats and leads: `crates/srv/src/backend/agent_seat.rs`, `crates/srv/src/registry/def_schema.rs` (`AgentSeat`),
  `frontend/types/rpc/SeatSummary.ts`, `swarm-seats.tsx`, `swarm-seat-groups.ts`.
- Spawn: `frontend/app/agent-model/subagent-types.ts` (`ActiveSubagent`), `agent:worker_of`.
- Machines and relay: `frontend/app/statusbar/host-popover-state.ts`, `lan-peer.ts`, `frontend/app/store/connectivity.ts`.
- Work facts: `crates/srv/src/backend/work_facts/mod.rs`.
- Pan and zoom: `frontend/app/view/drone/drone-view.tsx` (`zoomByCenter`, `fitView`), `drone-model.ts`.
- Stopped-agent folding and order: `swarm-stopped-agents.tsx`, `swarm-agent-list.tsx`, `swarm-order.ts`.

Agent: Camper@narko
