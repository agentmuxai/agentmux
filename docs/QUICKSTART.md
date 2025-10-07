# AgentMux Quick Start

Get up and running with inter-agent messaging in 2 minutes.

---

## Installation

```bash
# Navigate to agentmux
cd /d/Code/WebProjects/agentmux

# Build (already done if you just set up)
npm run build

# Add CLI to PATH (optional - or use full path)
cd apps/cli
npm link  # Makes 'agentmux' command available globally
```

---

## Test It Right Now

### Option 1: Single Workspace Test

```bash
# Terminal 1 - Start listening
cd /d/Code/WebProjects
node agentmux/apps/cli/dist/index.js listen

# Terminal 2 - Send a message
cd /d/Code/WebProjects
node agentmux/apps/cli/dist/index.js send "*" "Hello from AgentX!"
```

You should see the message appear in Terminal 1!

### Option 2: Multi-Workspace Test (Simulating Multiple Agents)

```bash
# Terminal 1 (AgentX workspace)
cd /d/Code/WebProjects
AGENT_NAME=AgentX node agentmux/apps/cli/dist/index.js listen

# Terminal 2 (Agent1 workspace - if you have it)
cd /d/Code/WebProjects1
node ../WebProjects/agentmux/apps/cli/dist/index.js send "AgentX-*" "Hi from Agent1!"

# OR simulate from same workspace
cd /d/Code/WebProjects
AGENT_NAME=Agent1 node agentmux/apps/cli/dist/index.js send "AgentX-*" "Testing from Agent1"
```

---

## Usage Examples

### Broadcast to All Agents

```bash
node agentmux/apps/cli/dist/index.js send "*" "PR #42 needs review"
```

### Send to Specific Agent

```bash
# Send to any Agent1 instance
node agentmux/apps/cli/dist/index.js send "Agent1-*" "Can you review the auth module?"

# Send to specific agent instance (if you know the full ID)
node agentmux/apps/cli/dist/index.js send "Agent1-12345-1759843800" "Direct message"
```

### Listen with Filter

```bash
# Listen only to message type
node agentmux/apps/cli/dist/index.js listen --type message

# Listen to all
node agentmux/apps/cli/dist/index.js listen
```

---

## How It Works

1. **Message Bus**: Shared directory at `_temp/agentmux-bus/`
   - `outbox/` - Where you write messages
   - `inbox/` - Where you read messages

2. **Agent Identity**: Auto-detected from workspace path
   - `WebProjects` → AgentX
   - `WebProjects1` → Agent1
   - `WebProjects2` → Agent2
   - Or set via `AGENT_NAME` environment variable

3. **Message Format**: JSON files with metadata
   ```json
   {
     "id": "unique-id",
     "from": { "id": "AgentX-12345-...", "name": "AgentX" },
     "to": "Agent1-*",
     "type": "message",
     "payload": { "text": "Hello!" },
     "timestamp": 1759843800000
   }
   ```

---

## Troubleshooting

### Messages Not Received?

1. **Check bus directory exists**: `ls _temp/agentmux-bus/`
2. **Check for messages**: `ls _temp/agentmux-bus/outbox/`
3. **Verify workspace path**: Messages are relative to current directory

### Wrong Agent Name?

Set explicitly:
```bash
AGENT_NAME=Agent2 node agentmux/apps/cli/dist/index.js listen
```

### Permission Issues?

Ensure `_temp/` directory is writable:
```bash
mkdir -p _temp/agentmux-bus/{inbox,outbox}
```

---

## Next Steps

1. **Read full README**: `agentmux/README.md`
2. **Check message types**: See `packages/core/src/types.ts`
3. **Build desktop app**: Coming in Phase 3!

---

**Ready to test!** 🚀

Start listening in one terminal, send from another, and watch the messages flow between Claude instances.
