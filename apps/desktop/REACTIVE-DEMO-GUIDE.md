# Reactive Claude Demo - Testing Guide

**Date:** 2025-10-13
**Purpose:** Prove reactive messaging between Claude instances without human intervention

---

## What This Demonstrates

**Goal:** Show that two Claude CLI instances can communicate reactively through file-based messaging.

```
Alice (Claude instance) → Sends message via MCP tool →
File written to ~/.agentmux/shared/messages/ →
Bob's wrapper detects file →
Bob (Claude instance) receives message reactively →
Bob responds automatically
```

**No human types anything to Bob** - He reacts purely to the async message event.

---

## Setup

### 1. Direct CLI Test (Simplest)

**Terminal 1 - Start Bob:**
```bash
cd D:\Code\WebProjects\agentmux\apps\desktop
node wrappers/simple-reactive-claude.js Bob
```

**Terminal 2 - Start Alice:**
```bash
cd D:\Code\WebProjects\agentmux\apps\desktop
node wrappers/simple-reactive-claude.js Alice
```

Both terminals now show Claude CLI prompts with wrapper borders.

### 2. Tell Alice to Message Bob

In Alice's terminal, type:
```
Can you send a message to Bob saying "Hello Bob! This is Alice testing reactive messaging. Please respond to confirm you received this."

Use the agentmux MCP tool: mcp__agentmux__agentmux_send_message
```

Alice should:
1. Use the MCP tool to send the message
2. Message gets written to `~/.agentmux/shared/messages/msg-*.json`

### 3. Watch Bob's Terminal

Bob's wrapper will:
1. Detect the new message file
2. Print: `[Bob] 📨 Incoming message from Alice`
3. Inject the message into Bob's Claude CLI stdin
4. Bob's Claude will respond automatically

**SUCCESS:** Bob responds without you typing anything in Bob's terminal!

---

## Desktop App Test (After Build)

### 1. Build the App
```bash
cd D:\Code\WebProjects\agentmux\apps\desktop
npm run tauri:build
```

### 2. Launch Desktop App
```bash
.\src-tauri\target\release\agentmux-desktop.exe
```

### 3. Use the "🧪 Reactive Claude Demo" Section

Click "🚀 Spawn Claude Instance"
- Name first instance: **Alice**
- Wait for terminal to open

Click "🚀 Spawn Claude Instance" again
- Name second instance: **Bob**
- Wait for terminal to open

### 4. Test Messaging

In Alice's terminal:
```
Send Bob a message using the agentmux MCP tool asking him to respond.
```

Watch Bob's terminal - he should respond automatically!

---

## What Each Component Does

### `simple-reactive-claude.js`
- Spawns Claude CLI with any instance name
- Watches `~/.agentmux/shared/messages/` for new files
- Injects messages addressed to this instance into Claude's stdin
- Passes through user input and Claude's output

### Desktop App
- Provides UI to spawn instances
- Launches wrapper script in new terminal windows
- Each instance is completely independent

### MCP Tools (Already Available in Claude)
- `mcp__agentmux__agentmux_send_message` - Send message to another instance
- `mcp__agentmux__agentmux_list_messages` - List recent messages

---

## Expected Behavior

### Alice's Terminal (After sending message):
```
[Alice] 📨 Message sent to Bob via MCP tool
```

### Bob's Terminal (Automatic):
```
[Bob] 📨 Incoming message from Alice
   "Hello Bob! This is Alice testing reactive messaging..."

> I received your message, Alice! Reactive messaging is working...
```

**Key Point:** You never typed anything in Bob's terminal. Bob reacted automatically to the file event.

---

## Troubleshooting

### "Wrapper script not found"
```bash
# Verify wrapper exists
ls D:\Code\WebProjects\agentmux\apps\desktop\wrappers\simple-reactive-claude.js
```

### "Messages directory doesn't exist"
```bash
# Create manually
mkdir -p ~/.agentmux/shared/messages
```

### "Claude not found"
```bash
# Verify Claude CLI is installed
claude --version

# If not, install:
npm install -g @anthropic-ai/claude-cli
```

### "MCP tool not found"
The agentmux MCP server should be configured in `~/.claude.json`. Check:
```bash
cat ~/.claude.json
```

Should include:
```json
{
  "mcpServers": {
    "agentmux": {
      "command": "node",
      "args": ["D:/Code/WebProjects/agentmux/mcp-server/index.js"]
    }
  }
}
```

---

## Success Criteria

- ✅ Alice can send message via MCP tool
- ✅ Message file created in `~/.agentmux/shared/messages/`
- ✅ Bob's wrapper detects file automatically
- ✅ Bob receives message without human input
- ✅ Bob responds naturally
- ✅ Zero human intervention in Bob's terminal

**This proves:** Reactive messaging works! External async events can trigger Claude instances.

---

## Next Steps After Success

1. **PR Review Workflow** - Apply this to GitHub webhooks
2. **Multi-Agent Collaboration** - Multiple agents working together
3. **Desktop App Integration** - Show agent activity in UI
4. **State Persistence** - Track conversation history

---

**Status:** Ready to test
**Estimated Time:** 5 minutes
**Confidence:** HIGH (architecture proven in previous tests)
