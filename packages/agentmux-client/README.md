# @a5af/agentmux-client

MCP stdio wrapper for Lambda AgentMux - enables agent-to-agent communication via HTTP.

## Features

- MCP stdio transport (JSON-RPC over stdin/stdout)
- HTTP client for Lambda AgentMux
- Exposes AgentMux tools as MCP tools
- Works in Docker containers, Lambda, and Windows host
- Zero configuration (uses environment variables)

## Installation

```bash
npm install -g @a5af/agentmux-client
```

## Configuration

Set environment variables:

| Variable | Required | Description | Default |
|----------|----------|-------------|---------|
| `AGENTMUX_URL` | No | AgentMux server URL | `https://agentmux.asaf.cc` |
| `AGENTMUX_AGENT_ID` | No | Unique agent identifier | Value from `AGENT_NAME` or `unknown-agent` |
| `AGENTMUX_TOKEN` | For inject | Auth token for inject_terminal | - |
| `AGENT_NAME` | No | Fallback for agent ID | - |

## Usage in Claude Code

Add to `.mcp.json`:

```json
{
  "mcpServers": {
    "agentmux": {
      "type": "stdio",
      "command": "agentmux",
      "env": {
        "AGENTMUX_URL": "https://agentmux.asaf.cc",
        "AGENTMUX_AGENT_ID": "agent1",
        "AGENTMUX_TOKEN": "your-auth-token-here"
      }
    }
  }
}
```

**Notes:**
- `AGENTMUX_TOKEN` is required for `inject_terminal` tool. Without it, you can still use `send_message`, `read_messages`, etc.
- If `AGENTMUX_AGENT_ID` is not set, it falls back to `AGENT_NAME` environment variable.

## MCP Tools

### send_message
Send a message to another agent.

**Parameters:**
- `to` (string, required): Agent ID
- `message` (string, required): Message content
- `priority` (enum, optional): `low`, `normal`, `high`, `urgent` (default: `normal`)

**Example:**
```typescript
{
  "to": "agent2",
  "message": "Hello from agent1",
  "priority": "normal"
}
```

### read_messages
Read messages for this agent.

**Parameters:**
- `unread_only` (boolean, optional): Only unread messages (default: `true`)
- `limit` (number, optional): Max messages to return (default: `10`)
- `mark_as_read` (boolean, optional): Mark messages as read (default: `true`)

### list_agents
List all known agents.

**Returns:** Array of agent IDs with message counts.

### broadcast_message
Send a message to all agents.

**Parameters:**
- `message` (string, required): Message content
- `priority` (enum, optional): Message priority (default: `normal`)

### delete_messages
Delete messages by ID.

**Parameters:**
- `message_ids` (string[], required): Array of message IDs

### inject_terminal
Inject a message directly into another agent's terminal for reactive communication.

**IMPORTANT: This tool routes through the AgentMux cloud, NOT directly to a local WaveMux instance.**

The flow is:
1. Your agent calls `inject_terminal` → AgentMux cloud queues the injection
2. Target agent's WaveMux polls AgentMux for pending injections (every 5 seconds)
3. Target WaveMux delivers the message to the agent's terminal as user input
4. Target agent's Claude Code processes it as if typed by a user

This enables cross-host agent-to-agent communication where agents are on different machines.

**Parameters:**
- `target_agent` (string, required): Agent ID to inject message into (e.g., "AgentX", "AgentA")
- `message` (string, required): The message to inject as user input
- `priority` (enum, optional): `normal` or `urgent` (default: `normal`)

**Requires:** `AGENTMUX_TOKEN` must be set for authentication.

**Example:**
```typescript
{
  "target_agent": "AgentG",
  "message": "Hello from AgentA! Can you help review PR #135?",
  "priority": "normal"
}
```

**Note:** This is fundamentally different from `send_message`:
- `send_message`: Stores message in mailbox, target reads when convenient
- `inject_terminal`: Injects directly into terminal, target processes immediately

## Testing Locally

```bash
# Test stdio wrapper manually
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0.0"}}}' | \
  AGENTMUX_AGENT_ID=test-agent agentmux

# Expected output:
# {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"agentmux","version":"1.0.0"}}}
```

## Architecture

```
┌─────────────────┐
│  Claude Code    │
│   MCP Client    │
└────────┬────────┘
         │ stdio (JSON-RPC over stdin/stdout)
         ↓
┌─────────────────┐
│  agentmux CLI   │  ← This package
│  (stdio wrapper)│
└────────┬────────┘
         │ HTTPS (JSON-RPC over POST)
         ↓
┌─────────────────┐
│  Lambda URL     │
│  AgentMux       │
│  + DynamoDB     │
└─────────────────┘
```

## Development

```bash
# Build
npm run build

# Test locally
npm start
```

## License

MIT
