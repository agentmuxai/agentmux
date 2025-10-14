# AgentMux Reactive Wrapper - Deployment Guide

**Status:** Ready for deployment
**Version:** 0.1.0
**Date:** 2025-10-12

## Overview

The AgentMux reactive wrapper enables supervised agent-to-agent communication by wrapping AI CLIs (like Claude Code) with real-time message notifications.

## Prerequisites

All agents (Agent1-5) have already:
- ✅ Updated their WebProjects workspace to latest
- ✅ Updated agentmux submodule to commit a6ffa88
- ✅ All tests passing (26/26)

## Deployment Steps

### Step 1: Build AgentMux Packages (Already Done in WebProjects)

```bash
cd /d/Code/WebProjects/agentmux
npm run build
```

Expected output:
```
Tasks: 5 successful, 5 total
```

### Step 2: Install node-pty (Already Done in WebProjects)

```bash
cd /d/Code/WebProjects/agentmux/apps/wrapper
npm install node-pty
```

### Step 3: Set Up Shared Messages Directory

Each agent workspace needs the shared messages directory:

```bash
# Create shared directory
mkdir -p ~/.agentmux/shared/messages

# Set permissions (owner only)
chmod 700 ~/.agentmux/shared
chmod 700 ~/.agentmux/shared/messages
```

**Windows (Git Bash/WSL):**
```bash
mkdir -p /c/Users/$USER/.agentmux/shared/messages
# Or in WSL: mkdir -p ~/.agentmux/shared/messages
```

### Step 4: Link AgentMux CLI Globally (Per Workspace)

Each agent needs to link the CLI from their workspace:

**For Agent1 (in WebProjects1):**
```bash
cd /d/Code/WebProjects1/agentmux/apps/cli
npm link
```

**For Agent2 (in WebProjects2):**
```bash
cd /d/Code/WebProjects2/agentmux/apps/cli
npm link
```

**Repeat for Agent3, Agent4, Agent5**

### Step 5: Test Wrapper Installation

```bash
# Check agentmux CLI is available
agentmux --version
# Expected: 0.1.0

# Check wrap command exists
agentmux wrap --help
```

## Usage

### Starting the Wrapper

**Basic:**
```bash
agentmux wrap claude
```

**With Agent ID (recommended):**
```bash
AGENT_ID=Agent1 agentmux wrap claude
# Or with flag:
agentmux wrap claude --agent-id Agent1
```

**With Debug Logging:**
```bash
agentmux wrap claude --agent-id Agent1 --debug
```

### What Happens

1. Wrapper spawns Claude CLI in PTY
2. File watcher monitors `~/.agentmux/shared/messages/`
3. When message arrives for this agent:
   - Blue notification appears on terminal
   - "check messages" command auto-injected
   - Human sees everything and can intervene

## Testing

### Test 1: Wrapper Starts Successfully

**Terminal (Agent1):**
```bash
cd /d/Code/WebProjects1
AGENT_ID=Agent1 agentmux wrap claude
```

**Expected:**
```
🔄 Starting wrapper for claude...
  Agent ID: Agent1

[Claude CLI starts normally]
```

### Test 2: Send Message Between Agents

**Terminal 1 (Agent1):**
```bash
agentmux wrap claude --agent-id Agent1
```

**Terminal 2 (Agent2 sends message):**
```bash
cd /d/Code/WebProjects2/agentmux
node -e "
const { MessageBus } = require('./packages/core/dist/message-bus.js');
const bus = new MessageBus({ id: 'Agent2', name: 'Agent2' });
bus.send('Agent1', 'message', { text: 'Hello from Agent2' });
"
```

**Expected in Terminal 1:**
```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 📨 Remote message from Agent2
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

check messages
```

### Test 3: MCP Integration (Next Step)

After wrapper is working, configure Claude Code to use MCP tools:

```json
// claude_desktop_config.json
{
  "mcpServers": {
    "agentmux": {
      "command": "node",
      "args": ["/d/Code/WebProjects/agentmux/apps/mcp-server/dist/index.js"],
      "env": {
        "AGENT_ID": "Agent1"
      }
    }
  }
}
```

## Verification Checklist

For each agent (Agent1-5), verify:

- [ ] agentmux submodule updated to a6ffa88
- [ ] `npm run build` successful in agentmux
- [ ] node-pty installed
- [ ] Messages directory created: `~/.agentmux/shared/messages/`
- [ ] CLI linked globally: `agentmux --version` works
- [ ] Wrapper starts: `agentmux wrap claude` launches
- [ ] Can send/receive test messages
- [ ] MCP server configured (separate step)

## Troubleshooting

### "agentmux: command not found"

**Solution:**
```bash
cd /d/Code/WebProjects<N>/agentmux/apps/cli
npm link
```

### "Cannot find module 'node-pty'"

**Solution:**
```bash
cd /d/Code/WebProjects/agentmux/apps/wrapper
npm install node-pty
```

### "PTY process not initialized"

**Cause:** Wrapper failed to start

**Solution:**
1. Check Claude CLI is installed: `claude --version`
2. Run with debug: `agentmux wrap claude --debug`
3. Check error logs

### Messages Not Detected

**Solution:**
```bash
# Check directory exists
ls -la ~/.agentmux/shared/messages/

# Test file watcher manually
agentmux wrap claude --debug
# In another terminal:
echo '{"id":"test","from":{"id":"Test"},"to":"Agent1","payload":{"text":"test"}}' > ~/.agentmux/shared/messages/test-123.json
```

### Wrapper Exits Immediately

**Cause:** Claude CLI not found or configuration issue

**Solution:**
1. Verify Claude CLI installed
2. Check PATH includes Claude
3. Run with debug logging

## Next Steps

After deployment:

1. **Test Message Flow:** Send test messages between agents
2. **Configure MCP:** Set up MCP server in Claude Code config
3. **End-to-End Test:** Full workflow with MCP + wrapper
4. **Document Workflows:** Create agent communication patterns
5. **Monitor Performance:** Track notification latency (<100ms target)

## Reference Documentation

- **Wrapper README:** `agentmux/apps/wrapper/README.md`
- **MCP Setup:** `agentmux/docs/MCP_SERVER_SETUP.md`
- **Core API:** `agentmux/packages/core/README.md`

## Support

- **PR #5:** https://github.com/a5af/agentmux/pull/5 (merged)
- **PR #157:** https://github.com/a5af/WebProjects/pull/157 (pending merge)
- **Test Results:** All 26 tests passing

---

**Deployment Status:** ✅ Ready
**Next Action:** Test wrapper with Agent1
