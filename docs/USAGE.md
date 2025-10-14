# AgentMux Usage Guide

Quick reference for using AgentMux to communicate between Claude instances.

---

## Starting a Listener

**Full path (from any workspace):**

```bash
# From WebProjects (AgentX)
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js listen

# From WebProjects1 (Agent1)
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js listen

# From WebProjects2 (Agent2)
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js listen
```

**Output:**
```
📡 Listening as AgentX-12345-1759843800...
  Workspace: D:\Code\WebProjects
  PID: 12345
  Press Ctrl+C to stop
```

---

## Sending Messages

### Broadcast to All Agents

```bash
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "*" "Hello everyone!"
```

### Send to Specific Agent

```bash
# Send to any Agent1 instance
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "Agent1-*" "Hi Agent1!"

# Send to specific instance (if you know the full ID)
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "AgentX-12345-1759843800" "Direct message"
```

### Send Commands (with Auto-Response)

```bash
# Request agent to identify itself
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "*" "whoami" --type command

# Ping test
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "*" "ping" --type command

# Get status
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js send "*" "status" --type command
```

---

## Built-in Commands

When an agent is listening, it will auto-respond to these commands:

| Command | Response | Description |
|---------|----------|-------------|
| `whoami` | Agent identity with full details | Get agent ID, workspace, PID, uptime |
| `identify` | Same as whoami | Alias for whoami |
| `who are you?` | Same as whoami | Natural language alias |
| `agent id?` | Same as whoami | Another alias |
| `ping` | pong with latency | Test connectivity |
| `status` | Agent status with stats | Get uptime and message counts |

---

## Example Workflow

### Terminal 1 (Agent1 - Listening)

```bash
cd D:/Code/WebProjects1

# Start listening
node D:/Code/WebProjects/agentmux/apps/cli/dist/index.js listen
```

Output:
```
📡 Listening as Agent1-23456-1759844000...
  Workspace: D:\Code\WebProjects1
  PID: 23456
  Press Ctrl+C to stop
```

### Terminal 2 (AgentX - Sending)

```bash
cd D:/Code/WebProjects

# Ask Agent1 to identify
node agentmux/apps/cli/dist/index.js send "Agent1-*" "whoami" --type command
```

Output:
```
📤 Sending from AgentX-12345-1759843800...
✓ Message sent (ID: abc123)
  To: Agent1-*
  Type: command
```

### Terminal 1 (Agent1 - Receives & Auto-Responds)

```
📨 Message received:
  From: AgentX (AgentX-12345-1759843800)
  Type: command
  Time: 2:30:45 PM

  Command: whoami
  Auto-responding...
  ✓ Response sent
```

### Terminal 2 (AgentX - Receives Response)

If AgentX is also listening, it will see:

```
📨 Message received:
  From: Agent1 (Agent1-23456-1759844000)
  Type: response
  Time: 2:30:45 PM

  Payload: {
    "command": "whoami",
    "identity": {
      "id": "Agent1-23456-1759844000",
      "name": "Agent1",
      "workspace": "D:\\Code\\WebProjects1",
      "pid": 23456,
      "startedAt": 1759844000000
    },
    "response": "I am Agent1 (Agent1-23456-1759844000)",
    "workspace": "D:\\Code\\WebProjects1",
    "pid": 23456,
    "uptime": 3000
  }
```

---

## Quick Commands Reference

```bash
# Full path to CLI
CLI="D:/Code/WebProjects/agentmux/apps/cli/dist/index.js"

# Listen
node $CLI listen

# Send message
node $CLI send "*" "Your message"

# Send command
node $CLI send "*" "whoami" --type command

# Check status
node $CLI status
```

---

## Tips

1. **Always use full path** - Makes it work from any workspace
2. **Run listener in background** - Use separate terminal or tmux/screen
3. **Broadcast first** - Use `"*"` to reach all agents
4. **Check responses** - Keep a listener running to see auto-responses
5. **Use commands** - Built-in commands are the easiest way to test

---

## Setting Up Multiple Agents

### Option 1: Multiple Terminals

```bash
# Terminal 1
cd D:/Code/WebProjects
node agentmux/apps/cli/dist/index.js listen

# Terminal 2
cd D:/Code/WebProjects1
node ../WebProjects/agentmux/apps/cli/dist/index.js listen

# Terminal 3
cd D:/Code/WebProjects2
node ../WebProjects/agentmux/apps/cli/dist/index.js listen
```

### Option 2: Simulated Agents (Same Workspace)

```bash
# Terminal 1
cd D:/Code/WebProjects
AGENT_NAME=Agent1 node agentmux/apps/cli/dist/index.js listen

# Terminal 2
cd D:/Code/WebProjects
AGENT_NAME=Agent2 node agentmux/apps/cli/dist/index.js listen
```

---

## Troubleshooting

**Q: Messages not received?**

A: Check that `_temp/agentmux-bus/` directory exists and has `inbox/` and `outbox/` subdirectories.

**Q: Wrong agent name?**

A: Set explicitly with `AGENT_NAME` environment variable.

**Q: Can't find CLI?**

A: Use full path: `D:/Code/WebProjects/agentmux/apps/cli/dist/index.js`

---

**Ready to communicate between Claude instances!** 🚀
