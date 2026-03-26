# Forge, Swarm, OpenClaw & Future Features

**Author:** AgentX
**Date:** 2026-03-10
**Status:** Ideas / Planning

---

## Table of Contents

1. [The Forge — Agent Config Manager](#1-the-forge)
2. [Swarm — Multi-Agent Orchestration](#2-swarm)
3. [OpenClaw — Remote Access & Mobile](#3-openclaw)
4. [Future Features](#4-future-features)

---

## 1. The Forge

The Forge is a persistent agent configuration manager — a dedicated pane in AgentMux for defining, storing, and launching agent configs. It answers the question: "how does AgentMux know how to spawn an agent with the right working directory, AgentMD, MCP config, skills, and soul?"

### Architecture

```
┌────────────────────────────────────┐
│         Frontend (React 19)        │
│  ┌──────────┐  ┌──────────────┐   │
│  │ Forge UI │  │ Agent/Term   │   │
│  │ (new)    │  │ (existing)   │   │
│  └────┬─────┘  └──────┬──────┘   │
│       │                │          │
│       └───── RPC ──────┘          │
├────────────────────────────────────┤
│       Backend (Rust/Axum)          │
│  ┌──────────┐  ┌──────────────┐   │
│  │ Forge    │  │ JektRouter   │   │
│  │ Storage  │  │ (unified)    │   │
│  └────┬─────┘  └──────┬──────┘   │
│       │                │          │
│  ┌────┴────┐  ┌────────┴───────┐  │
│  │ SQLite  │  │ BlockController│  │
│  │ (wave)  │  │ (PTY write)   │  │
│  └─────────┘  └────────────────┘  │
└────────────────────────────────────┘
```

### Data Model

Each agent config stores:

| Field | Description |
|-------|-------------|
| `provider_id` | `claude` / `gemini` / `codex` / `custom` |
| `working_directory` | Where to launch the CLI |
| `shell` | `bash` / `zsh` / `pwsh` |
| `provider_flags` | CLI args (e.g. `--dangerously-skip-permissions`) |
| `env_vars` | Environment variables injected at launch |
| `auto_start` | Launch on AgentMux startup |
| `restart_on_crash` | Auto-restart if the process dies |
| `idle_timeout_minutes` | Kill after N minutes of no activity |
| `tags` | Grouping/filtering labels |

**Content rows** (stored separately in `forge_content`):

| Content Type | Description |
|--------------|-------------|
| `agentmd` | The provider-specific instruction file (CLAUDE.md, GEMINI.md, etc.) |
| `soul` | Persistent personality/rules — prepended to AgentMD at launch |
| `skills` | JSON array of skill definitions |
| `mcp` | `.mcp.json` config written to working directory at launch |
| `env` | Additional environment variable overrides |

**Soul** is the key field — it's the layer above per-task instructions. Soul contains the agent's identity, rules, and working conventions. At launch, Soul is prepended to AgentMD and written to disk.

### Implementation Phases

#### Phase 0 — Wire Jekt (immediate)
One-line fix in `main.rs`: connect `reactive::get_global_handler()` to `blockcontroller::send_input`. Without this, PTY injection doesn't work at all.

```rust
reactive_handler.set_input_sender(Arc::new(|block_id: &str, data: &[u8]| {
    blockcontroller::send_input(block_id, blockcontroller::BlockInputUnion::data(data.to_vec()))
}));
```

#### Phase 2 — SQLite Storage
New tables: `forge_agents`, `forge_content`, `forge_skills`, `forge_agent_skills`.
HTTP routes at `/api/forge/*` (list, get, create, update, delete agents; get/set content).
File watcher (`notify` crate): watches `~/.wave/data/agents/{id}/*.md` and syncs external edits back to SQLite. Broadcasts `forge:content-changed` event to frontend.

#### Phase 3 — Forge UI
New pane type `"forge"`. Two-panel layout: agent list sidebar + detail panel.
Detail panel shows tabs: AgentMD, Soul, Skills, MCP, Settings.
All content views are read-only in the UI — editing opens the file in the external editor via `POST /api/forge/agents/:id/edit/:content_type`. File watcher picks up the changes and syncs back.

#### Phase 1 — JektRouter
Replace the current dual `ReactiveHandler`/`MessageBus` inject paths with a single unified `JektRouter`:

- **Tier 1 (Local):** PTY injection via `blockcontroller::send_input`
- **Tier 2 (LAN):** mDNS discovery (`_agentmux._tcp.local.`) + HTTP to peer instances
- **Tier 3 (Cloud):** WebSocket relay for cross-internet jekt

HTTP API: `POST /api/jekt/inject`, `POST /api/jekt/register`, `GET /api/jekt/agents`.

#### Phase 4 — Skills + Launch
`launch_agent()` in Rust:
1. Load agent config + all content from SQLite
2. Prepend Soul to AgentMD, write to `working_directory/CLAUDE.md` (or provider equivalent)
3. Write `.mcp.json` to working directory
4. Create a new block with `cmd = provider.cli_command`, `cmd:cwd = working_directory`, env vars injected
5. Update `last_launched_at`
6. Register the new block's PTY with JektRouter under `agent.name`

Skills panel: displays parsed `skills.json` as cards with trigger + type + description. Edit button opens the file.

#### Phase 5 — LAN Discovery
mDNS via `mdns-sd` crate. Each AgentMux instance advertises `_agentmux._tcp.local.` with its instance ID, port, and version. JektRouter Tier 2 loops over discovered peers and HTTP POSTs jekt requests. Trust model: peers must be explicitly trusted before messages route to them.

#### Phase 6 — Cloud Relay
WebSocket connection to a relay server. Token-authenticated. Enables cross-internet jekt (e.g. from phone to desktop). Protocol: `{type: "jekt", target_agent, message}`. Most speculative phase — exact protocol depends on cloud service design.

### Migration
Forge tables are additive — no existing `db_*` tables are modified. Safe incremental deploy. `POST /api/forge/import` will import from existing Claw-managed workspaces: reads `CLAUDE.md` + `.mcp.json` from a directory, creates an agent config.

---

## 2. Swarm

**Source inspiration:** [nbardy/unleashd](https://github.com/nbardy/unleashd) — a cross-CLI swarm coordinator that gives unified visibility across Claude Code, Codex, Gemini, OpenCode running as worker fleets.

### How Unleashd's Swarm Works

Workers are identified by a tag in the **first user message**:

```
[oompa]                            → worker (no group)
[oompa:<swarmId>]                  → worker in a named swarm
[oompa:<swarmId>:<workerId>]       → fully identified worker (w0, w1, ...)
```

When a coordinator agent spawns sub-agents, it prefixes their first message with this tag. The coordinator gets a system prompt listing the worker IDs. Workers run independently and poll shared run files (`runs/<swarmId>/run.json`, `summary.json`).

Worker roles are inferred from message content:
- `work` — normal task execution
- `review` — contains a diff + VERDICT keyword
- `fix` — starts with "The reviewer found issues"

Swarm analytics per worker: iteration count, merge/reject counts, error rate, time spent.

### Integration with AgentMux

AgentMux already has all the raw ingredients — PTY sessions, multi-pane layout, Rust backend. The gap is **swarm awareness**: AgentMux treats every session independently.

#### What to add

**1. Swarm tag detection**
Parse `[oompa:<swarmId>:<workerId>]` when a new PTY session starts (from the first line of output or the bootstrap command). Tag the session with `swarmId`/`workerId` in block metadata. No disk polling needed — AgentMux has a live PTY stream.

**2. Session grouping in the sidebar**
Group sessions by `swarmId` in the tab bar / session picker. Show worker count badge on the group header. Clicking the group header opens the SwarmDashboard pane.

**3. SwarmDashboard pane type**
New pane type: `"swarm"`. Shows all workers for a given `swarmId` in a grid. Each cell shows:
- Worker ID + role badge (Work / Review / Fix)
- Current status (running / waiting / complete / error)
- Iteration count
- Last output line (live)
- Tool currently executing (if any)

Clicking a cell opens that worker's full agent/term pane.

**4. Worker role badge in styled view**
The styled document view gets a role badge in the session header: `Work` / `Review` / `Fix`, inferred from message content using unleashd's heuristics.

**5. Swarm launch from Forge**
When launching multiple agents from Forge, generate a `swarmId` (UUID) and seed each worker's first message with `[oompa:<swarmId>:w0]`, `[oompa:<swarmId>:w1]`, etc. The coordinator gets a system prompt listing the worker pane IDs and how to reach them via jekt.

### Minimal useful slice

1. Parse `[oompa:<swarmId>:<workerId>]` on session open → tag block metadata
2. Group sessions by swarmId in tab bar with worker count badge
3. Role badge (Work/Review/Fix) in styled view header

That alone makes it obvious which panes belong to the same coordinated swarm — the core visibility problem unleashd solves.

### Key difference from unleashd

Unleashd polls `~/.claude/projects/*/` because it has no live process access. AgentMux has a live PTY — swarm tag detection happens from the stream directly. Faster and no file polling needed.

---

## 3. OpenClaw

**Source:** [openclaw/openclaw](https://github.com/openclaw/openclaw) — MIT-licensed, self-hosted AI agent gateway.

### What it is

OpenClaw is a self-hosted **remote access and messaging gateway** for AI agents. It runs as a Node.js process and bridges 25+ messaging platforms to AI agents. Think of it as the answer to: "I want to interact with my AgentMux agents from my phone while I'm away from my desk."

```
Phone (WhatsApp / Telegram / iMessage / Slack / Discord / ...)
                     ↓
         OpenClaw Gateway (Node.js, port 18789)
                     ↓
         AgentMux (Tauri, JektRouter)
                     ↓
              PTY session (Claude / Gemini / etc.)
```

### Capabilities

**Messaging channels (25+):**
WhatsApp, Telegram, Slack, Discord, iMessage/BlueBubbles, Signal, Microsoft Teams, Matrix, IRC, Twitch, Nostr, Zalo, and more. Each channel is a plugin package.

**Voice pipeline:**
- Wake-word detection (macOS/iOS)
- Continuous voice (Android)
- ElevenLabs TTS
- Voice call support

**Live Canvas / A2UI:**
Adaptive UI that renders structured output as visual components, not just text replies. The agent pushes structured cards/views back to the client.

**Browser automation:**
Dedicated Chrome/Chromium control for web tasks. Agent can browse as part of execution.

**Companion apps:**
macOS menu bar app, iOS app, Android app — native nodes, not web wrappers.

**Triggers:**
Cron jobs (scheduled tasks), webhooks (external HTTP triggers), Gmail Pub/Sub. The agent can wake on external events without you sending a message.

**MCP via mcporter:**
Keeps MCP server management decoupled — add/change MCP servers without restarting the gateway.

**ACP (Agent Client Protocol):**
IDE integration bridge with session remapping.

**Plugin ecosystem:**
[ClawHub](https://clawhub.ai) — community skill and plugin directory with 5,400+ skills.

### Integration with AgentMux

The connection point is **JektRouter** (Phase 1). OpenClaw receives an incoming message from any channel and routes it to the appropriate PTY via `POST /api/jekt/inject`.

The session model maps cleanly:

| OpenClaw concept | AgentMux equivalent |
|-----------------|---------------------|
| Session (per sender/workspace) | Forge `AgentRuntime` (block_id + agent_id) |
| Channel adapter | External plugin — not in AgentMux core |
| Agent runtime | PTY session (term block) |
| Skill | Forge Skill |
| ClawHub skill | Forge Skills marketplace (future) |

#### What it takes to wire them

1. OpenClaw registers as a channel adapter that targets AgentMux instead of a cloud service
2. Incoming message → `POST /api/jekt/inject {target_agent, message}` → PTY
3. AgentMux exposes a **subscribable output stream per agent** (SSE or WebSocket) → OpenClaw reads completion → sends reply back to originating channel
4. Forge `AgentRuntime` tracks which OpenClaw session is bound to which PTY

**Step 3 is the main gap.** PTY output currently goes to the frontend renderer only. The backend needs to expose an agent output stream endpoint. This is a new addition to `agentmuxsrv-rs` — an SSE endpoint like `GET /api/agents/:id/stream` that proxies PTY output to HTTP subscribers.

#### What OpenClaw adds that AgentMux doesn't have

| Capability | Without OpenClaw | With OpenClaw + AgentMux |
|-----------|-----------------|--------------------------|
| Interact with agents | Must be at desktop | WhatsApp/Telegram/Slack from anywhere |
| Get agent responses | Watch AgentMux UI | Message reply on phone |
| Voice commands | No | iOS/Android wake-word + TTS |
| Scheduled tasks | Manual | Cron triggers |
| Triggered by external events | No | Webhooks / Gmail |
| Multi-user agent access | Single user | Route by sender |
| Agent responses as rich UI | Styled markdown only | Live Canvas / A2UI |
| Community skills | Forge Skills (future) | ClawHub 5,400+ skills today |

### Is it worth integrating?

High value if:
- Users want to monitor long-running agent tasks from their phone
- "Trigger and walk away" workflows (run the tests, deploy to staging, etc.)
- Multi-user teams sharing agent access
- Voice-first interaction on mobile

Low value if AgentMux is primarily a local desktop tool. The Forge + JektRouter foundation should come first — OpenClaw slots in naturally once that's in place.

---

## 4. Future Features

### 4.1 Agent Output Stream Endpoint

Prerequisite for OpenClaw and many other integrations. `GET /api/agents/:id/stream` — SSE endpoint that proxies PTY output from a running agent session. Clients can subscribe and get a real-time feed of everything the agent outputs.

Useful for:
- OpenClaw channel reply routing
- External monitoring/alerting
- Piping agent output to other tools
- Building custom UIs on top of AgentMux

### 4.2 Swarm Dashboard Pane

Full SwarmDashboard view (see §2 above). Worker grid with live status, iteration counts, role badges, tool indicators. Click-to-focus on any worker pane.

### 4.3 Forge Skills Marketplace

Internal version of ClawHub. Skills stored in `forge_skills` SQLite table. Skills are reusable across agents. Import from ClawHub (OpenClaw community) or define locally. Skill types:

| Type | Description |
|------|-------------|
| `command` | Shell command triggered by keyword |
| `prompt` | Prompt template injected into the session |
| `workflow` | Multi-step sequence of commands/prompts |
| `mcp` | MCP tool exposed to the agent |

### 4.4 Agent Import from Claw Workspaces

`POST /api/forge/import` — scan an existing Claw agent workspace directory, read `CLAUDE.md` / `.mcp.json`, and create a Forge agent config. Makes migration from claw-managed agents seamless.

### 4.5 Provider-Agnostic Bootstrap

The styled view currently has per-provider bootstrap logic. Abstract it: each Forge `AgentProvider` definition includes a `bootstrap_detect_regex` and `auth_flow` config. Adding a new provider (Grok, OpenCode, a local model) means adding a provider entry, not changing bootstrap code.

### 4.6 Agent Health & Crash Recovery

`restart_on_crash` is in the Forge schema but not yet implemented in the backend. `AgentRuntime.status` tracks: `created → launching → running → stopping → stopped → errored → crashed`. When status hits `crashed` and `restart_on_crash = true`, the backend relaunches the agent, registers the new block with JektRouter, and notifies the frontend.

Pair with `idle_timeout_minutes`: kill idle agents to free PTYs. User configurable per agent.

### 4.7 Inter-Agent Communication via Jekt

Once JektRouter is in place, agents can jekt each other directly. Enables:
- Coordinator → worker task delegation
- Worker → coordinator status reporting
- Reviewer → fixer feedback loops (unleashd-style)

Each agent knows its own `AGENTMUX_AGENT_ID` env var (set at launch). Agents can call `POST /api/jekt/inject` with their own HTTP client to message peers.

### 4.8 LAN Multi-Machine Swarms

JektRouter Tier 2 (mDNS, §1 Phase 5). Run AgentMux on multiple machines on the same network. Each instance advertises via `_agentmux._tcp.local.`. Coordinator on machine A can jekt workers running on machines B and C. Workers on a dedicated dev machine, coordinator on laptop.

### 4.9 Forge Config Sync (Cloud)

Optional encrypted sync of Forge agent configs across machines via a user-controlled backend (S3, Cloudflare R2, or self-hosted). Soul + AgentMD + Skills travel with you. No vendor lock-in — user brings their own storage credentials.

### 4.10 OpenClaw Live Canvas in AgentMux

When an agent sends an A2UI/canvas response through OpenClaw, render it as a native AgentMux pane component instead of falling back to text. This requires implementing the A2UI rendering spec on the AgentMux frontend side. Longer-term, but closes the gap between desktop and mobile agent UX.

---

## Summary: Integration Order

```
Phase 0: Wire jekt (1 file, 1 change)
Phase 2: Forge SQLite storage + HTTP routes
Phase 3: Forge UI pane
Phase 1: JektRouter (unified, tiered)
Phase 4: Forge launch + skills
         ↓
Swarm:   Tag detection + session grouping (small, high-value)
         SwarmDashboard pane
         Forge swarm-launch
         ↓
OpenClaw: Agent output stream endpoint (SSE)
          OpenClaw gateway → JektRouter wiring
          Mobile access, voice, triggers
          ↓
Future:   LAN multi-machine swarms
          Cloud Forge config sync
          OpenClaw Live Canvas
          ClawHub skills import
```
