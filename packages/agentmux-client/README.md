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
| `AGENTMUX_URL` | No | Lambda Function URL | `https://xv7wycacd3vmglr7j24cfdkhb40buykg.lambda-url.us-east-1.on.aws` |
| `AGENTMUX_AGENT_ID` | No | Unique agent identifier | Value from `AGENT_NAME` or `unknown-agent` |
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
        "AGENTMUX_URL": "https://xv7wycacd3vmglr7j24cfdkhb40buykg.lambda-url.us-east-1.on.aws",
        "AGENTMUX_AGENT_ID": "agent1"
      }
    }
  }
}
```

**Note:** If `AGENTMUX_AGENT_ID` is not set, it falls back to `AGENT_NAME` environment variable.

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
