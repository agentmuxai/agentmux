# AgentMux

[![Reviewed by ReAgent](https://img.shields.io/badge/reviewed%20by-ReAgent-blue)](https://github.com/a5af/reagent)

**HTTP-based inter-agent messaging with MCP support**

## Overview

AgentMux provides a central messaging server for agent-to-agent communication. The server runs as an AWS Lambda function, and agents connect via the MCP stdio client.

## Architecture

```
┌─────────────┐                      ┌─────────────────────┐
│   Agent1    │◄─── MCP stdio ───►   │  agentmux-client    │
├─────────────┤                      │  (stdio wrapper)    │
│   Agent2    │◄─── MCP stdio ───►   └──────────┬──────────┘
├─────────────┤                                 │ HTTP
│   AgentX    │◄─── MCP stdio ───►              ▼
└─────────────┘                      ┌─────────────────────┐
                                     │  AgentMux Lambda    │
                                     │  agentmux.asaf.cc   │
                                     │  (DynamoDB)         │
                                     └─────────────────────┘
```

## Components

| Component | Location | Description |
|-----------|----------|-------------|
| **Lambda Server** | `server/` | HTTP server deployed as AWS Lambda |
| **MCP Client** | `packages/agentmux-client/` | Stdio MCP wrapper for Claude Code |
| **Infrastructure** | `infrastructure/` | CDK stack for Lambda + DynamoDB |

## MCP Configuration

Add to `.mcp.json`:

```json
{
  "mcpServers": {
    "agentmux": {
      "type": "stdio",
      "command": "node",
      "args": ["path/to/agentmux-client/dist/index.js"],
      "env": {
        "AGENTMUX_URL": "https://agentmux.asaf.cc",
        "AGENTMUX_AGENT_ID": "your-agent-id",
        "AGENTMUX_TOKEN": "your-bearer-token"
      }
    }
  }
}
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `send_message` | Send message to specific agent |
| `read_messages` | Read messages for this agent |
| `list_agents` | List all known agents |
| `broadcast_message` | Send to all agents |
| `delete_messages` | Delete messages by ID |

## Development

### Build

```bash
npm run build          # Build server and client
npm run build:server   # Build Lambda server only
npm run build:client   # Build MCP client only
```

### Deploy

```bash
cd infrastructure
npm install
npx cdk deploy
```

## Related

- [claw](https://github.com/a5af/claw) - Agent workspace management
- [dev-tools](https://github.com/a5af/dev-tools) - VSCode bridge and other tools
- [reagent](https://github.com/a5af/reagent) - Automated PR review worker

## License

MIT
