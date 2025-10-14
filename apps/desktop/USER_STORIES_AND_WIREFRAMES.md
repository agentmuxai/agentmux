# AgentMux Desktop - User Stories & UI Wireframes

**Version:** 0.3.1
**Date:** 2025-10-14
**Purpose:** Complete UI documentation with text wireframes and user stories

---

## Table of Contents

1. [Overview](#overview)
2. [View 1: Dashboard](#view-1-dashboard)
3. [View 2: Bus Control](#view-2-bus-control)
4. [View 3: Agents Manager](#view-3-agents-manager)
5. [View 4: Message Stream](#view-4-message-stream)
6. [Cross-View Features](#cross-view-features)
7. [User Journey Maps](#user-journey-maps)

---

## Overview

AgentMux Desktop is a native desktop application for monitoring and orchestrating multiple Claude AI agent instances through a centralized message bus.

**Core Capabilities:**
- Start/stop a WebSocket message bus for agent communication
- Spawn and manage multiple Claude agent instances
- Monitor real-time message flow between agents
- View agent terminal output and logs
- Debug console for system-level logs

---

## View 1: Dashboard

### Text Wireframe

```
╔═══════════════════════════════════════════════════════════════════════════════╗
║ 🤖 AgentMux Desktop                    [🚀 Dashboard] [🔌 Bus] [🤖 Agents] [💬]║
╠═══════════════════════════════════════════════════════════════════════════════╣
║                                                                               ║
║ ┌─ Server Bus Control ──────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  ● Status: Stopped                                                         │║
║ │                                                                            │║
║ │  [▶️ Start Bus]  [⏹️ Stop Bus]                                             │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
║ ┌────────────────┐  ┌────────────────┐  ┌────────────────────────────────┐  ║
║ │ Connected      │  │ Messages/sec   │  │ Bus Status                     │  ║
║ │ Agents         │  │                │  │                                │  ║
║ │                │  │                │  │                                │  ║
║ │      0         │  │      0         │  │            X                   │  ║
║ │   Offline      │  │   0 total      │  │   ws://localhost:8765          │  ║
║ └────────────────┘  └────────────────┘  └────────────────────────────────┘  ║
║                                                                               ║
║ ┌─ Recent Activity ──────────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  Start the bus to begin monitoring agents.                                │║
║ │                                                                            │║
║ │  💡 Tip: Go to the Agents tab to spawn reactive Claude instances          │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
╠═══════════════════════════════════════════════════════════════════════════════╣
║ AgentMux v0.2.9  |  Built: 2025-10-13 6:45 AM PT  |  Status: Ready           ║
║ ▼ Debug Console (1)  [Clear] [Copy]                                          ║
║ 08:19:08.774  [LOG]  Command watcher started                                 ║
╚═══════════════════════════════════════════════════════════════════════════════╝
```

### User Stories

#### US-D1: Start Message Bus
**As a** developer orchestrating multiple agents
**I want to** start the centralized message bus
**So that** agents can connect and communicate with each other

**Acceptance Criteria:**
- [ ] Click "Start Bus" button
- [ ] Bus starts on ws://localhost:8765
- [ ] Status changes from "Stopped" to "Running"
- [ ] Bus Status card shows green checkmark
- [ ] Recent Activity shows "Bus started on ws://localhost:8765"
- [ ] Debug console logs bus startup event

**Technical Details:**
- Invokes: `start_bus` with config `{ host: '127.0.0.1', port: 8765, max_agents: 50 }`
- Listens for: `bus_started` event
- Updates metrics every 2 seconds

---

#### US-D2: Stop Message Bus
**As a** developer
**I want to** stop the message bus
**So that** I can shut down the agent infrastructure cleanly

**Acceptance Criteria:**
- [ ] Click "Stop Bus" button
- [ ] Bus stops gracefully
- [ ] Status changes to "Stopped"
- [ ] Connected Agents resets to 0
- [ ] Messages/sec resets to 0
- [ ] Recent Activity shows "Bus stopped"

**Technical Details:**
- Invokes: `stop_bus`
- Listens for: `bus_stopped` event
- Disconnects all active agents

---

#### US-D3: Monitor Bus Metrics
**As a** developer
**I want to** see real-time metrics of the message bus
**So that** I can understand system health and activity

**Acceptance Criteria:**
- [ ] "Connected Agents" shows count of active agents
- [ ] "Messages/sec" shows throughput rate
- [ ] "Bus Status" shows WebSocket URL
- [ ] Metrics update every 2 seconds
- [ ] Metrics accurate within 500ms of actual state

**Technical Details:**
- Polls: `get_bus_status` every 2000ms
- Returns: `{ running: bool, agents_connected: number, messages_per_second: number, total_messages: number }`

---

#### US-D4: View Recent Activity
**As a** developer
**I want to** see a chronological feed of system events
**So that** I can understand what's happening in the system

**Acceptance Criteria:**
- [ ] Recent Activity shows latest 20 events
- [ ] Events include: bus start/stop, agent connect/disconnect, errors
- [ ] Events show timestamps
- [ ] Events auto-scroll (newest at top)
- [ ] Helpful tips shown when bus is stopped

---

## View 2: Bus Control

### Text Wireframe

```
╔═══════════════════════════════════════════════════════════════════════════════╗
║ 🤖 AgentMux Desktop                    [🚀 Dashboard] [🔌 Bus] [🤖 Agents] [💬]║
╠═══════════════════════════════════════════════════════════════════════════════╣
║                                                                               ║
║ ┌─ Bus Configuration ────────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  Host:          [127.0.0.1                           ]                    │║
║ │  Port:          [8765      ]                                              │║
║ │  Max Agents:    [50        ]                                              │║
║ │                                                                            │║
║ │  [▶️ Start Bus with Custom Config]                                        │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
║ ┌─ Connection Status ────────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  WebSocket URL: ws://127.0.0.1:8765                                       │║
║ │  Status:        🟢 Running                                                │║
║ │  Uptime:        00:15:32                                                  │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
║ ┌─ Connected Agents (3) ─────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  • agent-12345-1759800000  │  Connected: 5m ago  │  Messages: 142         │║
║ │  • agent-67890-1759800100  │  Connected: 3m ago  │  Messages: 89          │║
║ │  • agent-11111-1759800200  │  Connected: 1m ago  │  Messages: 23          │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
║ ┌─ Message Log (Last 50) ────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  08:25:15  agent-12345 → agent-67890  │  "Task completed"                │║
║ │  08:25:14  agent-67890 → agent-12345  │  "Processing request..."         │║
║ │  08:25:10  agent-11111 → *            │  "Broadcast: Status update"      │║
║ │                                                                            │║
║ │  [Pause Stream] [Clear Log] [Export Log]                                  │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
╠═══════════════════════════════════════════════════════════════════════════════╣
║ AgentMux v0.2.9  |  Built: 2025-10-13 6:45 AM PT  |  Status: Ready           ║
╚═══════════════════════════════════════════════════════════════════════════════╝
```

### User Stories

#### US-B1: Configure Bus Settings
**As a** developer
**I want to** configure bus host, port, and capacity
**So that** I can customize the infrastructure for my use case

**Acceptance Criteria:**
- [ ] Can specify custom host (default: 127.0.0.1)
- [ ] Can specify custom port (default: 8765)
- [ ] Can set max agents limit (default: 50)
- [ ] Settings persist across sessions
- [ ] Validation prevents invalid configs

---

#### US-B2: View Connected Agents List
**As a** developer
**I want to** see all agents currently connected to the bus
**So that** I can understand which agents are active

**Acceptance Criteria:**
- [ ] List shows agent ID, connection time, message count
- [ ] List updates in real-time as agents connect/disconnect
- [ ] Can click agent to view details
- [ ] Shows agent status (connected, idle, busy)

---

#### US-B3: Monitor Message Flow
**As a** developer
**I want to** see messages flowing through the bus
**So that** I can debug agent communication

**Acceptance Criteria:**
- [ ] Shows last 50 messages
- [ ] Displays: timestamp, from agent, to agent, message preview
- [ ] Auto-scrolls with new messages
- [ ] Can pause stream
- [ ] Can filter by agent ID
- [ ] Can export log to file

---

## View 3: Agents Manager

### Text Wireframe

```
╔═══════════════════════════════════════════════════════════════════════════════╗
║ 🤖 AgentMux Desktop                    [🚀 Dashboard] [🔌 Bus] [🤖 Agents] [💬]║
╠═══════════════════════════════════════════════════════════════════════════════╣
║                                                                               ║
║ ┌─ Spawn New Agent ──────────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  Workspace Path:  [D:\Code\WebProjects\myproject          ] [📁 Browse]   │║
║ │  Agent Label:     [myproject                              ]               │║
║ │  Command:         [claude                                 ] ⓘ            │║
║ │                                                                            │║
║ │  [🚀 Spawn Agent]                                                         │║
║ │                                                                            │║
║ │  💡 Tip: Agent will run in selected workspace directory                   │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
║ ┌─ Active Agents (2) ────────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  ┌─ myproject ─────────────────────────────────────────────────────────┐  │║
║ │  │  PID: 12345  │  Port: 9999  │  Started: 15m ago                     │  │║
║ │  │  Status: 🟢 Running                                                  │  │║
║ │  │  [View Terminal] [Stop Agent] [Restart]                             │  │║
║ │  └──────────────────────────────────────────────────────────────────────┘  │║
║ │                                                                            │║
║ │  ┌─ askbase ──────────────────────────────────────────────────────────┐  │║
║ │  │  PID: 67890  │  Port: 9998  │  Started: 8m ago                      │  │║
║ │  │  Status: 🟢 Running                                                  │  │║
║ │  │  [View Terminal] [Stop Agent] [Restart]                             │  │║
║ │  └──────────────────────────────────────────────────────────────────────┘  │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
║ ┌─ Agent Terminal: myproject ────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  [embedded_claude] myproject - Process spawned with PID: 12345            │║
║ │  [embedded_claude] myproject - Starting WebSocket server on port 9999...  │║
║ │  WebSocket server listening on 127.0.0.1:9999                             │║
║ │  [WS:127.0.0.1:54321] ✓ WebSocket connected                               │║
║ │  [WS:127.0.0.1:54321] ← Received text message #1: 'hello' (5 bytes)       │║
║ │  [WS:127.0.0.1:54321] → Forwarding to stdin channel...                    │║
║ │  [WS:127.0.0.1:54321] ✓ Successfully sent to stdin channel                │║
║ │  [myproject:stdin] → Sending input #1 (5 bytes)                           │║
║ │  [myproject:stdin] ✓ Input #1 sent successfully                           │║
║ │  [myproject:stdout] Claude's response here...                             │║
║ │                                                                            │║
║ │  [Send Input...                                    ] [Send]               │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
╠═══════════════════════════════════════════════════════════════════════════════╣
║ AgentMux v0.2.9  |  Built: 2025-10-13 6:45 AM PT  |  Status: Ready           ║
╚═══════════════════════════════════════════════════════════════════════════════╝
```

### User Stories

#### US-A1: Spawn New Agent
**As a** developer
**I want to** spawn a new Claude agent instance in a specific workspace
**So that** I can have Claude work on that project

**Acceptance Criteria:**
- [ ] Click "Browse" to select workspace directory
- [ ] Agent label auto-fills from folder name
- [ ] Can customize agent label
- [ ] Can customize command (default: "claude")
- [ ] Click "Spawn Agent" creates new instance
- [ ] New agent appears in Active Agents list
- [ ] Agent starts with WebSocket server on random port
- [ ] Success message shows agent details (PID, port)

**Technical Details:**
- Invokes: `spawn_claude_instance({ label, workspace_path, command })`
- Returns: `{ instance_name, pid, ws_port }`
- Starts embedded Claude Code CLI in specified directory
- Creates WebSocket bridge for UI communication

---

#### US-A2: View Agent Terminal Output
**As a** developer
**I want to** see real-time terminal output from an agent
**So that** I can monitor its activity and debug issues

**Acceptance Criteria:**
- [ ] Click "View Terminal" on an agent
- [ ] Terminal panel shows stdout/stderr
- [ ] Output streams in real-time
- [ ] Shows comprehensive logging from v0.3.1 (WebSocket messages, stdin forwarding, etc.)
- [ ] Auto-scrolls to newest output
- [ ] Can scroll up to view history
- [ ] Handles ANSI color codes

**Technical Details:**
- Invokes: `get_agent_output({ agent_id })`
- Polls every 500ms for new output
- Displays formatted log messages

---

#### US-A3: Send Input to Agent
**As a** developer
**I want to** send text input to a running agent
**So that** I can interact with Claude through the UI

**Acceptance Criteria:**
- [ ] Input field at bottom of terminal
- [ ] Type message and press Enter (or click Send)
- [ ] Message sent to agent's stdin
- [ ] Message appears in terminal output
- [ ] Logs show WebSocket forwarding steps (v0.3.1 logging)
- [ ] Claude response appears in terminal
- [ ] Input field clears after sending

**Technical Details:**
- WebSocket connection to `ws://localhost:{agent.ws_port}`
- Sends text message via WebSocket
- Forwarded through stdin channel to Claude subprocess
- Response comes back through stdout stream

---

#### US-A4: Stop Agent
**As a** developer
**I want to** stop a running agent
**So that** I can free up resources

**Acceptance Criteria:**
- [ ] Click "Stop Agent" button
- [ ] Agent process terminates gracefully
- [ ] Agent removed from Active Agents list
- [ ] WebSocket connections closed
- [ ] Confirmation dialog prevents accidental stops

**Technical Details:**
- Invokes: `stop_claude_instance({ instance_name })`
- Sends SIGTERM to process
- Force-kills after 5s timeout if needed

---

#### US-A5: Restart Agent
**As a** developer
**I want to** restart an agent
**So that** I can recover from errors without manual respawn

**Acceptance Criteria:**
- [ ] Click "Restart" button
- [ ] Agent stops gracefully
- [ ] New instance spawns with same config
- [ ] Terminal clears and shows new session
- [ ] Preserves workspace path and label

---

#### US-A6: Browse Workspace Directory
**As a** developer
**I want to** use a file browser to select workspace
**So that** I don't have to type paths manually

**Acceptance Criteria:**
- [ ] Click "Browse" button
- [ ] Native file picker dialog opens
- [ ] Can navigate filesystem
- [ ] Can only select directories (not files)
- [ ] Selected path fills workspace field
- [ ] Agent label auto-suggests from folder name

**Technical Details:**
- Uses: `@tauri-apps/plugin-dialog.open()`
- Requires: `dialog:allow-open` ACL permission (fixed in v0.3.1)

---

## View 4: Message Stream

### Text Wireframe

```
╔═══════════════════════════════════════════════════════════════════════════════╗
║ 🤖 AgentMux Desktop                    [🚀 Dashboard] [🔌 Bus] [🤖 Agents] [💬]║
╠═══════════════════════════════════════════════════════════════════════════════╣
║                                                                               ║
║ ┌─ Message Controls ─────────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  Filter: [agent-12345                    ]  Max: [100  ▼]  [⏸ Pause]     │║
║ │  [🗑️ Clear All]  [📥 Export to File]                                      │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
║ ┌─ Live Message Stream (45 messages) ────────────────────────────────────────┐║
║ │                                                                            │║
║ │  ┌────────────────────────────────────────────────────────────────────┐   │║
║ │  │ 14:32:15  agent-12345 → agent-67890            Priority: normal     │   │║
║ │  │                                                                     │   │║
║ │  │ "Can you review the code in src/components/Dashboard.tsx?"         │   │║
║ │  │                                                                     │   │║
║ │  │ [↩️ Reply]  [📋 Copy]                                               │   │║
║ │  └────────────────────────────────────────────────────────────────────┘   │║
║ │                                                                            │║
║ │  ┌────────────────────────────────────────────────────────────────────┐   │║
║ │  │ 14:32:10  agent-67890 → agent-12345            Priority: high       │   │║
║ │  │                                                                     │   │║
║ │  │ "Sure, I'll take a look at that file now."                         │   │║
║ │  │                                                                     │   │║
║ │  │ [↩️ Reply]  [📋 Copy]                                               │   │║
║ │  └────────────────────────────────────────────────────────────────────┘   │║
║ │                                                                            │║
║ │  ┌────────────────────────────────────────────────────────────────────┐   │║
║ │  │ 14:31:55  agent-11111 → * (broadcast)         Priority: normal     │   │║
║ │  │                                                                     │   │║
║ │  │ "System checkpoint: All agents healthy"                            │   │║
║ │  │                                                                     │   │║
║ │  │ [↩️ Reply]  [📋 Copy]                                               │   │║
║ │  └────────────────────────────────────────────────────────────────────┘   │║
║ │                                                                            │║
║ │  ... (42 more messages)                                                   │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
║ ┌─ Reply to: agent-12345 ────────────────────────────────────────────────────┐║
║ │                                                                            │║
║ │  Original: "Can you review the code in src/components/Dashboard.tsx?"     │║
║ │                                                                            │║
║ │  Your Reply:                                                              │║
║ │  ┌────────────────────────────────────────────────────────────────────┐   │║
║ │  │ I found a few issues in that file...                               │   │║
║ │  │                                                                     │   │║
║ │  └────────────────────────────────────────────────────────────────────┘   │║
║ │                                                                            │║
║ │  [✉️ Send Reply]  [✖️ Cancel]                                             │║
║ │                                                                            │║
║ └────────────────────────────────────────────────────────────────────────────┘║
║                                                                               ║
╠═══════════════════════════════════════════════════════════════════════════════╣
║ AgentMux v0.2.9  |  Built: 2025-10-13 6:45 AM PT  |  Status: Ready           ║
╚═══════════════════════════════════════════════════════════════════════════════╝
```

### User Stories

#### US-M1: View Live Message Stream
**As a** developer
**I want to** see all messages flowing between agents in real-time
**So that** I can understand agent collaboration patterns

**Acceptance Criteria:**
- [ ] Shows messages in reverse chronological order (newest first)
- [ ] Displays: timestamp, from agent, to agent, message text, priority
- [ ] Auto-scrolls with new messages
- [ ] Can manually scroll to view older messages
- [ ] Distinguishes broadcast messages (to: *)
- [ ] Shows priority levels (normal, high, urgent)

**Technical Details:**
- Listens for: `agent_message` events
- Stores last N messages (configurable, default 100)
- Formats timestamps as HH:MM:SS

---

#### US-M2: Filter Messages
**As a** developer
**I want to** filter messages by agent ID or text content
**So that** I can focus on relevant communication

**Acceptance Criteria:**
- [ ] Filter input supports partial matching
- [ ] Filters by: from agent ID, to agent ID, message text, priority
- [ ] Case-insensitive search
- [ ] Results update in real-time as typing
- [ ] Shows count of filtered vs total messages

**Technical Details:**
- Client-side filtering on message array
- Searches: `from.id`, `from.name`, `to`, `priority`, `payload.text`

---

#### US-M3: Pause Message Stream
**As a** developer
**I want to** pause the live stream
**So that** I can read messages without them scrolling away

**Acceptance Criteria:**
- [ ] Click "Pause" button
- [ ] New messages stop being added to view
- [ ] Messages still collected in background
- [ ] Pause button shows "Resume"
- [ ] Resume button unfreezes stream

**Technical Details:**
- Sets `paused()` signal to true
- `addMessage()` function checks pause state before updating UI

---

#### US-M4: Reply to Message
**As a** developer
**I want to** reply to a specific message
**So that** I can communicate with agents through the UI

**Acceptance Criteria:**
- [ ] Click "Reply" on a message
- [ ] Reply panel opens showing original message
- [ ] Text area for composing reply
- [ ] Send button posts reply to bus
- [ ] Reply appears in message stream
- [ ] Original sender receives the message

**Technical Details:**
- Invokes: `send_message({ to: message.from.id, message: text, priority: 'normal' })`
- Message routed through bus to target agent

---

#### US-M5: Export Message Log
**As a** developer
**I want to** export messages to a file
**So that** I can analyze communication patterns offline

**Acceptance Criteria:**
- [ ] Click "Export to File" button
- [ ] File save dialog opens
- [ ] Can choose filename and location
- [ ] Exports as JSON or text format
- [ ] Includes all visible messages (respects filter)
- [ ] Includes timestamp, agents, content, priority

---

#### US-M6: Clear Message History
**As a** developer
**I want to** clear the message stream
**So that** I can start fresh monitoring

**Acceptance Criteria:**
- [ ] Click "Clear All" button
- [ ] Confirmation dialog appears
- [ ] After confirm, all messages removed from view
- [ ] Counter resets to 0
- [ ] New messages start appearing immediately

---

## Cross-View Features

### Text Wireframe: Debug Console (Global)

```
╔═══════════════════════════════════════════════════════════════════════════════╗
║ ... (any view above) ...                                                      ║
╠═══════════════════════════════════════════════════════════════════════════════╣
║ AgentMux v0.2.9  |  Built: 2025-10-13 6:45 AM PT  |  Status: Ready           ║
║ ▼ Debug Console (27)  [Clear] [Copy]                                         ║
║ ┌───────────────────────────────────────────────────────────────────────────┐ ║
║ │ 08:19:08.774  [LOG]   Command watcher started                            │ ║
║ │ 08:19:12.445  [LOG]   [IPC] Server started on port 61088                 │ ║
║ │ 08:19:15.123  [LOG]   [embedded_claude] myproject - Process spawned      │ ║
║ │ 08:19:15.890  [LOG]   [embedded_claude] myproject - WebSocket on 9999    │ ║
║ │ 08:19:20.234  [LOG]   [WS:127.0.0.1:54321] ✓ WebSocket connected         │ ║
║ │ 08:19:25.567  [LOG]   [WS:127.0.0.1:54321] ← Received text message #1    │ ║
║ │ 08:19:25.578  [LOG]   [WS:127.0.0.1:54321] ✓ Sent to stdin channel       │ ║
║ │ ... (20 more lines) ...                                                  │ ║
║ └───────────────────────────────────────────────────────────────────────────┘ ║
╚═══════════════════════════════════════════════════════════════════════════════╝
```

### User Stories

#### US-C1: View System Logs
**As a** developer
**I want to** see system-level debug logs
**So that** I can diagnose application issues

**Acceptance Criteria:**
- [ ] Console shows logs from: IPC server, command watcher, embedded Claude, WebSocket
- [ ] Logs include timestamp, log level, source, message
- [ ] Console collapsible (click ▼ to expand/collapse)
- [ ] Shows count of total log entries in header
- [ ] Auto-scrolls to newest log

---

#### US-C2: Clear Debug Console
**As a** developer
**I want to** clear the debug console
**So that** I can focus on new log entries

**Acceptance Criteria:**
- [ ] Click "Clear" button
- [ ] All logs removed from view
- [ ] Counter resets to 0
- [ ] New logs start appearing immediately

---

#### US-C3: Copy Logs
**As a** developer
**I want to** copy console logs to clipboard
**So that** I can share them for debugging

**Acceptance Criteria:**
- [ ] Click "Copy" button
- [ ] All visible logs copied to clipboard
- [ ] Format: plain text with timestamps
- [ ] Success notification appears

---

## User Journey Maps

### Journey 1: First-Time Setup

```
1. Launch App
   ↓
2. See Dashboard (bus stopped, 0 agents)
   ↓
3. Read tip: "Go to Agents tab to spawn instances"
   ↓
4. Click "Agents" tab
   ↓
5. Click "Browse" → select project directory
   ↓
6. Agent label auto-fills (e.g., "myproject")
   ↓
7. Click "Spawn Agent"
   ↓
8. Agent appears in Active Agents list
   ↓
9. Terminal shows agent startup logs
   ↓
10. Type "hello" in input → Press Enter
    ↓
11. See message flow in logs:
    - [WS] Received message
    - [WS] Forwarding to stdin
    - [stdin] Sending input
    - [stdout] Claude's response
    ↓
12. Success! Agent is working
```

**Duration:** ~2 minutes
**Pain Points:**
- Finding Claude CLI executable path (if not in PATH)
- Understanding WebSocket port assignment

**Improvements:**
- Auto-detect Claude CLI location
- Show WebSocket URL in agent card

---

### Journey 2: Multi-Agent Collaboration

```
1. Start on Dashboard
   ↓
2. Click "Start Bus"
   ↓
3. Bus starts → Status: Running
   ↓
4. Go to Agents tab
   ↓
5. Spawn Agent #1 (project-a)
   ↓
6. Spawn Agent #2 (project-b)
   ↓
7. Spawn Agent #3 (project-c)
   ↓
8. Go to Messages tab
   ↓
9. See agents registering with bus
   ↓
10. Agent #1 sends message to Agent #2
    ↓
11. Agent #2 responds to Agent #1
    ↓
12. Monitor message flow in real-time
    ↓
13. Click "Reply" on a message
    ↓
14. Send manual message from UI
    ↓
15. Agent receives and processes message
```

**Duration:** ~5 minutes
**Pain Points:**
- Managing many agent instances
- Finding specific messages in stream

**Improvements:**
- Agent grouping/tagging
- Advanced message filtering (by conversation thread)

---

### Journey 3: Debugging Communication Issues

```
1. User reports: "My agents aren't talking"
   ↓
2. Go to Dashboard
   ↓
3. Check: Is bus running? ✓ Yes
   ↓
4. Check: Connected Agents = 2 ✓ Correct
   ↓
5. Go to Messages tab
   ↓
6. Filter by agent ID
   ↓
7. See no messages in last 5 minutes
   ↓
8. Go to Agents tab
   ↓
9. Click "View Terminal" on Agent #1
   ↓
10. Check logs - see WebSocket errors:
    "Failed to connect to bus"
    ↓
11. Ah! Bus port mismatch
    ↓
12. Stop agent
    ↓
13. Check bus config (port 8765)
    ↓
14. Restart agent with correct port
    ↓
15. Terminal shows: "Connected to bus"
    ↓
16. Messages start flowing ✓ Fixed!
```

**Duration:** ~3 minutes
**Key Features Used:**
- Bus status monitoring
- Agent terminal logs
- WebSocket connection logging (v0.3.1)

---

## Summary Statistics

**Total Views:** 4
- Dashboard
- Bus Control
- Agents Manager
- Message Stream

**Total User Stories:** 22
- Dashboard: 4 stories
- Bus Control: 3 stories
- Agents Manager: 6 stories
- Message Stream: 6 stories
- Cross-View: 3 stories

**Key User Personas:**
1. **Solo Developer** - Managing agents for personal projects
2. **Team Lead** - Orchestrating multiple agents for team workflow
3. **DevOps Engineer** - Monitoring agent infrastructure health

**Critical User Flows:**
1. Spawn agent → Send input → View response
2. Start bus → Monitor messages → Debug issues
3. Multi-agent → Message filtering → Export logs

---

## Implementation Priority

### P0 - Must Have (Core Experience)
- [ ] US-A1: Spawn New Agent
- [ ] US-A2: View Agent Terminal Output
- [ ] US-A3: Send Input to Agent
- [ ] US-D1: Start Message Bus
- [ ] US-C1: View System Logs

### P1 - Should Have (Enhanced UX)
- [ ] US-A6: Browse Workspace Directory
- [ ] US-M1: View Live Message Stream
- [ ] US-D3: Monitor Bus Metrics
- [ ] US-B2: View Connected Agents List
- [ ] US-C2: Clear Debug Console

### P2 - Nice to Have (Power User Features)
- [ ] US-M4: Reply to Message
- [ ] US-M5: Export Message Log
- [ ] US-B3: Monitor Message Flow
- [ ] US-A5: Restart Agent
- [ ] US-M2: Filter Messages

### P3 - Future Enhancements
- Agent grouping/tagging
- Message threading
- Performance metrics graphs
- Custom agent templates
- Saved workspace presets

---

**Document Version:** 1.0
**Last Updated:** 2025-10-14
**Status:** Complete
