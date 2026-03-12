# OpenClaw Widget Spec

## Overview

A first-class AgentMux widget. If OpenClaw is already running, opens directly
to its live dashboard — all existing sessions, channels, and history immediately
visible. If not installed, walks through install + environment setup.

After setup, OpenClaw replaces `a5af/claw` as the agent session manager and
adds external messaging channel connectivity.

---

## State machine

```
Open widget
     │
     ▼
Probe localhost:18789 (OpenClaw gateway)
  ├─ 200 OK → Screen 3: WebView dashboard (full live state, zero friction)
  └─ no response
         │
         ▼
    Is `openclaw` installed? (which/where openclaw)
    ├─ YES → Screen 2b: "Start OpenClaw"
    └─ NO  → Screen 1: Install + setup
```

**Key design principle:** existing OpenClaw users land on Screen 3 immediately —
their channels, sessions, conversation history, skills all preserved. The install
flow only runs for first-time users.

---

## Screen 1 — Not installed

- OpenClaw logo + one-line description
- Platform-detected install command shown (read-only, for transparency):
  - Windows: `iwr -useb https://openclaw.ai/install.ps1 | iex`
  - macOS/Linux: `curl -fsSL https://openclaw.ai/install.sh | bash`
- "Install OpenClaw" primary CTA button
- On click: spawn shell running installer with `--no-onboard` flag; stream
  output into pane; on exit 0 → advance to Screen 2a (first-time setup)

---

## Screen 2a — First-time environment setup

Collect minimum config before starting the gateway:

| Field | Source / Notes |
|---|---|
| AI provider | Dropdown: Claude / OpenAI / Gemini / OpenRouter |
| API key | Password input; pre-fill from `ANTHROPIC_API_KEY` env if present |
| Agent name | Pre-fill from `AGENTMUX_AGENT_ID` env var if set |
| Messaging channel | Optional: Telegram / WhatsApp / Discord / Skip for now |

On submit:
1. `openclaw config set provider <provider>`
2. `openclaw config set apiKey <key>`
3. `openclaw start` (launch gateway)
4. Advance to Screen 3 (WebView)

---

## Screen 2b — Installed but not running

- Status indicator: "OpenClaw installed, gateway not running"
- "Start OpenClaw" button → runs `openclaw start` → advance to Screen 3
- Link to openclaw docs

---

## Screen 3 — Live dashboard (WebView)

`WebView` pointing at `http://localhost:18789` (OpenClaw's Control UI).

For existing users opening the widget for the first time: their channels, agents,
conversation history, running skills all appear immediately — zero state migration,
zero re-configuration.

AgentMux passes the `controlUi.allowedOrigins` CORS check automatically since
the WebView origin is `http://localhost`.

Gateway port is `18789` by default; respects `OPENCLAW_GATEWAY_PORT` env var.

---

## Messaging layers (widget context)

The OpenClaw widget surfaces external channel connectivity. It does NOT replace
AgentMux's inter-pane MCP tools — these solve different problems:

| Layer | Where | Tools |
|---|---|---|
| Inter-pane (peer agents) | Inside AgentMux desktop | `send_message`, `inject_terminal`, `broadcast_message`, `list_agents`, `read_messages` |
| External channels (humans) | Via OpenClaw widget | `message` tool: Telegram, WhatsApp, Discord, Slack, iMessage, Signal… |
| Agent spawning | Via OpenClaw ACP | Parent→child hierarchical; not peer-to-peer |

An agent can use all three simultaneously:
- AgentMux MCP → coordinate with AgentB in the next pane
- OpenClaw `message` → notify the user on Telegram
- OpenClaw ACP → spin up a focused subagent for a subtask

---

## Implementation plan

### 1. Widget entry (`agentmuxsrv-rs/src/config/widgets.json`)
```json
"defwidget@openclaw": {
    "display:order": 10,
    "display:label": "OpenClaw",
    "icon": "lobster",
    "blockdef": { "meta": { "view": "openclaw" } }
}
```

### 2. View registration (`frontend/app/block/block.tsx`)
```typescript
import { OpenClawViewModel } from "@/app/view/openclaw/openclaw-model";
BlockRegistry.set("openclaw", OpenClawViewModel);
```

### 3. ViewModel (`frontend/app/view/openclaw/openclaw-model.ts`)

State atom: `"checking" | "running" | "installed" | "not-installed" | "installing" | "setup"`

On construct:
1. HTTP probe `localhost:18789` → if 200, set `"running"`
2. Else: `which openclaw` / `where openclaw` → `"installed"` or `"not-installed"`

### 4. View component (`frontend/app/view/openclaw/openclaw-view.tsx`)

```
"checking"      → spinner
"running"       → <WebView src="http://localhost:18789" />
"installed"     → Screen 2b
"not-installed" → Screen 1
"installing"    → log stream from controller output
"setup"         → Screen 2a form
```

### 5. Platform detection

```typescript
import { platform } from "@tauri-apps/plugin-os";
const installCmd = (await platform()) === "windows"
  ? `powershell -Command "iwr -useb https://openclaw.ai/install.ps1 | iex"`
  : `curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-onboard`;
```

### 6. Install execution

Reuse existing shell controller — spawn a block running the install command,
subscribe to controller output subject for log streaming, watch for exit code.

---

## File layout

```
frontend/app/view/openclaw/
  openclaw-model.ts     ViewModel + state atom
  openclaw-view.tsx     Screen switching + all 3 screens
  openclaw-view.scss    Styles
```

---

## Configuration reference

| Item | Value |
|---|---|
| Gateway port | `18789` (default); `OPENCLAW_GATEWAY_PORT` env override |
| Config file | `~/.openclaw/openclaw.json` |
| State dir | `~/.openclaw/` |
| Memory store | `~/.openclaw/memory/<agentId>.sqlite` |

---

## Out of scope (v1)

- Bundling OpenClaw as a sidecar inside AgentMux portable ZIP
- Forge-level features (Docker agent management, multi-workspace orchestration)
- OpenClaw auto-update UI
- Multi-account / multi-gateway support
- Deep context engine integration (compaction/memory) — tracked separately
