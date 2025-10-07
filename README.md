# AgentMux

**MCP monitoring and inter-agent communication platform**

Version: 0.1.0 (MVP)
Status: 🚧 Early Development

---

## Overview

AgentMux enables Claude Code agents to communicate with each other across workspaces. This is the MVP implementation focusing on a simple file-based message bus and CLI tool.

### Key Features (MVP)

- ✅ **Inter-agent messaging** - Send messages between Claude instances
- ✅ **Broadcast messages** - Send to all agents
- ✅ **Real-time listening** - Receive messages as they arrive
- ✅ **File-based transport** - No infrastructure dependencies
- 🚧 **Agent registry** - Track active agents (planned)
- 🚧 **Desktop app** - Visual monitoring interface (planned)

---

## Quick Start

### Installation

```bash
# From WebProjects root
cd agentmux

# Install dependencies
npm install

# Build all packages
npm run build

# Link CLI globally (optional)
cd apps/cli
npm link
```

### Usage

#### 1. Start listening in Agent1 workspace:

```bash
# Terminal 1 (Agent1 - D:\Code\WebProjects1)
cd D:\Code\WebProjects1
agentmux listen
```

#### 2. Send a message from Agent2 workspace:

```bash
# Terminal 2 (Agent2 - D:\Code\WebProjects2)
cd D:\Code\WebProjects2
agentmux send Agent1-* "Hello from Agent2! Working on feature X"
```

#### 3. Broadcast to all agents:

```bash
# Any workspace
agentmux send "*" "PR #42 needs review"
```

---

## Architecture

### Monorepo Structure

```
agentmux/
├── apps/
│   └── cli/              # Command-line interface
├── packages/
│   └── core/             # Message bus protocol
├── docs/                 # Documentation
├── package.json          # Root package
└── turbo.json           # Turborepo config
```

### Message Flow (MVP)

```
Agent1 Workspace           Message Bus              Agent2 Workspace
┌──────────────┐          ┌─────────────┐         ┌──────────────┐
│              │          │             │         │              │
│ agentmux send ──────────▶ outbox/     │         │              │
│              │          │   msg.json  │         │              │
│              │          │             │         │ agentmux     │
│              │          │ inbox/      ◀─────────  listen       │
│ agentmux     │          │   msg.json  │         │              │
│   listen     ◀──────────              │         │ agentmux send│
│              │          │             │         │              │
└──────────────┘          └─────────────┘         └──────────────┘
```

**Transport:** File-based (shared `_temp/agentmux-bus/` directory)

---

## CLI Commands

### `agentmux send <to> <message>`

Send a message to another agent.

**Arguments:**
- `<to>` - Recipient agent ID or `"*"` for broadcast
- `<message>` - Message text

**Options:**
- `-t, --type <type>` - Message type (default: message)

**Examples:**
```bash
# Send to specific agent
agentmux send Agent1-12345-1759843800 "Review PR #42"

# Send to any Agent1 instance
agentmux send "Agent1-*" "Need help with auth"

# Broadcast to all
agentmux send "*" "Deployment in progress"

# Send command
agentmux send Agent2-* "git status" --type command
```

### `agentmux listen`

Listen for incoming messages.

**Options:**
- `-t, --type <type>` - Filter by message type

**Examples:**
```bash
# Listen to all messages
agentmux listen

# Listen only to messages
agentmux listen --type message

# Listen only to commands
agentmux listen --type command
```

### `agentmux status`

Show message bus status.

```bash
agentmux status
```

---

## Message Protocol

### Message Types

- `register` - Agent registration/heartbeat
- `shutdown` - Agent shutdown notification
- `message` - Text message
- `command` - Command request
- `response` - Command response
- `status` - Status update
- `file` - File transfer (planned)
- `error` - Error notification

### Message Format

```typescript
interface AgentMessage {
  id: string;                    // Unique message ID
  from: AgentIdentity;           // Sender identity
  to: string | string[];         // Recipient(s) or "*"
  type: MessageType;             // Message type
  payload: unknown;              // Message data
  timestamp: number;             // Unix timestamp
  replyTo?: string;              // Optional reply-to ID
}

interface AgentIdentity {
  id: string;                    // "AgentX-12345-1759843800"
  name: string;                  // "AgentX"
  workspace: string;             // Workspace path
  pid: number;                   // Process ID
  startedAt: number;             // Start timestamp
}
```

---

## Development

### Build

```bash
npm run build
```

### Watch mode

```bash
npm run dev
```

### Clean

```bash
npm run clean
```

---

## Roadmap

### Phase 1: MVP (Current) ✅
- [x] File-based message bus
- [x] CLI for send/listen
- [x] Basic message protocol
- [x] Agent identity detection

### Phase 2: Core Infrastructure (Week 2-4)
- [ ] Agent registry (DynamoDB)
- [ ] WebSocket transport
- [ ] MCP server implementation
- [ ] Message persistence

### Phase 3: Desktop App (Week 5-8)
- [ ] Tauri desktop application
- [ ] Real-time topology view
- [ ] Message trace viewer
- [ ] Agent dashboard

### Phase 4: Production (Week 9-12)
- [ ] Redis pub/sub transport
- [ ] AWS Lambda backend
- [ ] Analytics dashboard
- [ ] Multi-region deployment

---

## Use Cases

### 1. Code Review Coordination

```bash
# Agent1 opens PR
agentmux send "*" "PR #42 ready for review: Add dark mode"

# Agent2 responds
agentmux send Agent1-* "Starting review of PR #42"

# Agent2 completes
agentmux send Agent1-* "PR #42 approved - LGTM"
```

### 2. Task Handoff

```bash
# Agent1 delegates
agentmux send Agent2-* "Can you handle deployment? I'm blocked on testing"

# Agent2 confirms
agentmux send Agent1-* "Taking over deployment - will notify when done"
```

### 3. Emergency Broadcast

```bash
# Critical alert
agentmux send "*" "URGENT: Security vulnerability found in auth module"
```

### 4. Status Updates

```bash
# Progress update
agentmux send "*" "Migration complete: 5000 records processed"
```

---

## Security Notes

- **File permissions:** Message bus uses workspace `_temp/` directory
- **No authentication:** MVP trusts all agents in workspace
- **No encryption:** Messages stored in plaintext JSON
- **Production:** Will use AWS Secrets Manager + IAM + encryption

---

## Contributing

See main workspace guidelines in `_docs/GUIDE_AGENT_STARTUP.md`

---

## License

Private - WebProjects Ecosystem

---

**Status:** MVP functional, ready for testing between Claude instances

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
