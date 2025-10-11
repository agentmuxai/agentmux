# AgentMux MCP Server Setup Guide

## Overview

The AgentMux MCP server enables Claude Code to automatically receive and react to inter-agent messages. When a message arrives for your agent, Claude Code will be notified and can use MCP tools to respond.

## Prerequisites

- AgentMux Phase 1 installed (shared bus + global executable)
- Claude Code with MCP support
- Node.js 20+

## Installation

### 1. Verify MCP Server Built

```bash
ls ~/.agentmux/bin/agentmux-mcp
# Should exist and be executable
```

If not, rebuild agentmux:
```bash
cd /d/Code/WebProjects/agentmux
npm install
npm run build
```

### 2. Configure Claude Code

Create or update your Claude Code configuration file at:
- **Windows**: `C:\Users\<username>\.claude\config.json`
- **macOS/Linux**: `~/.claude/config.json`

Add the agentmux MCP server:

```json
{
  "mcpServers": {
    "agentmux": {
      "command": "node",
      "args": ["C:\\Users\\<username>\\.agentmux\\bin\\agentmux-mcp"],
      "env": {}
    }
  }
}
```

**On macOS/Linux**, use:
```json
{
  "mcpServers": {
    "agentmux": {
      "command": "node",
      "args": ["/Users/<username>/.agentmux/bin/agentmux-mcp"],
      "env": {}
    }
  }
}
```

### 3. Restart Claude Code

Restart Claude Code to load the MCP server.

## Available MCP Tools

Once configured, Claude Code will have access to these tools:

### `agentmux_send_message`

Send a message to another agent.

**Parameters:**
- `to` (string): Recipient agent ID, wildcard (e.g., "Agent1-*"), or "*" for broadcast
- `message` (string): Message text
- `type` (string, optional): Message type ("message", "command", "status")

**Example:**
```typescript
{
  "to": "Agent1-*",
  "message": "Please review PR #42",
  "type": "message"
}
```

### `agentmux_list_messages`

List recent messages from the bus.

**Parameters:**
- `limit` (number, optional): Max messages to return (default: 10)
- `type` (string, optional): Filter by message type

**Example:**
```typescript
{
  "limit": 5,
  "type": "message"
}
```

### `agentmux_reply`

Reply to a specific message.

**Parameters:**
- `messageId` (string): ID of message to reply to
- `reply` (string): Reply text

**Example:**
```typescript
{
  "messageId": "abc123",
  "reply": "I'll review it now"
}
```

### `agentmux_get_agents`

Get list of active agents (placeholder - registry not yet implemented).

## How It Works

### Message Flow

1. **Agent1 sends message**:
   ```bash
   agentmux send "AgentX-*" "Need help with deployment"
   ```

2. **MCP server detects message**:
   - Watches `~/.agentmux/shared/messages/` directory
   - Polls every 500ms for new messages
   - Filters for messages addressed to current agent

3. **Notification sent to Claude Code**:
   ```json
   {
     "method": "notifications/message",
     "params": {
       "level": "info",
       "data": {
         "type": "agentmux_message",
         "from": "Agent1",
         "message": "Need help with deployment",
         "messageId": "abc123"
       }
     }
   }
   ```

4. **Claude Code reacts**:
   - User sees notification in Claude Code UI
   - Claude can automatically use `agentmux_reply` tool
   - Or user can manually invoke tools

### Automatic Reactions

With MCP integration, Claude Code can:
- Automatically acknowledge messages
- Answer common questions (e.g., "whoami", "status")
- Coordinate work (e.g., "PR #42 ready for review")
- Notify user of important messages

## Testing

### Test 1: Verify MCP Server Starts

```bash
# Start MCP server manually (for debugging)
~/.agentmux/bin/agentmux-mcp

# Should output:
# [AgentMux MCP] Started as AgentX-<pid>-<timestamp>
# [AgentMux MCP] Watching: C:\Users\<username>\.agentmux\shared\messages
# [AgentMux MCP] Server connected and ready
```

Press Ctrl+C to stop.

### Test 2: Send Test Message from Another Agent

From Agent1 workspace:
```bash
cd /d/Code/WebProjects1
agentmux send "AgentX-*" "Test MCP notification"
```

If MCP server is running, you should see:
```
[AgentMux MCP] Notification sent for message <id>
```

And Claude Code should show a notification.

### Test 3: Use MCP Tools in Claude Code

In Claude Code, try:
- "List recent agentmux messages"
- "Send a message to Agent1 saying hello"
- "Reply to the last message"

Claude should use the corresponding MCP tools.

## Troubleshooting

### MCP Server Not Starting

**Error**: `Cannot find module`

**Solution**: Verify the path in config.json points to the correct location:
```bash
# Check if file exists
ls ~/.agentmux/bin/agentmux-mcp
```

### No Notifications Received

**Check 1**: MCP server running?
```bash
# From another terminal, send a test message
agentmux send "*" "Test broadcast"

# Check MCP server logs (if running in foreground)
# Should see: [AgentMux MCP] Notification sent for message <id>
```

**Check 2**: Messages in shared directory?
```bash
ls ~/.agentmux/shared/messages/
# Should see .json files
```

**Check 3**: Claude Code configuration?
- Verify `~/.claude/config.json` has correct MCP server config
- Restart Claude Code after config changes

### Tools Not Available

**Check**: MCP server in Claude Code tools list?
- In Claude Code, ask: "What MCP tools are available?"
- Should see: agentmux_send_message, agentmux_list_messages, etc.

## Next Steps

- **Phase 3**: Agent registry (track active agents, heartbeats)
- **Phase 4**: Advanced features (file transfer, encryption)
- **Phase 5**: Web dashboard for monitoring

## Support

For issues, check:
- AgentMux logs: `~/.agentmux/logs/` (if implemented)
- Claude Code logs
- GitHub issues: https://github.com/a5af/agentmux/issues
