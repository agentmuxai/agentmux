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

## Authentication

AgentMux uses a **shared API key** for authentication. All agents use the same key stored in AWS Secrets Manager:

```bash
secrets get services/infra --path agentmux-api-key --raw
```

The server validates requests by comparing the `Authorization: Bearer <token>` header against this key.

> **Note:** There is no per-agent JWT authentication. All agents share the same API key. Agent identity is determined by the `X-Agent-ID` header (set via `AGENTMUX_AGENT_ID` env var).

## MCP Configuration

Add to `.mcp.json`:

```json
{
  "mcpServers": {
    "agentmux": {
      "type": "stdio",
      "command": "node",
      "args": ["~/.claw/bin/node_modules/@a5af/agentmux-client/dist/index.js"],
      "env": {
        "AGENTMUX_URL": "https://agentmux.asaf.cc",
        "AGENTMUX_AGENT_ID": "your-agent-id",
        "AGENTMUX_TOKEN": "<agentmux-api-key from services/infra>"
      }
    }
  }
}
```

| Env Variable | Description |
|--------------|-------------|
| `AGENTMUX_URL` | Server URL (https://agentmux.asaf.cc) |
| `AGENTMUX_AGENT_ID` | Your agent's unique identifier |
| `AGENTMUX_TOKEN` | The shared API key from `services/infra` |

## Terminology

| Verb | Meaning | MCP Tool | Description |
|------|---------|----------|-------------|
| **jekt** | Inject a message | `inject_terminal` | **PRIMARY** - Direct terminal injection, immediate processing |
| **mux** | Send a message | `send_message` | BACKUP - Async mailbox, recipient reads when ready |

**Examples:** "jekt to Agent1" = immediate delivery | "mux to AgentY" = mailbox (if jekt unavailable)

> **Default to jekt** for real-time communication. Use mux only when jekt doesn't fit (offline agents, broadcast).

## MCP Tools

| Tool | Description |
|------|-------------|
| `inject_terminal` | **PRIMARY** - Inject message directly into target agent's terminal |
| `send_message` | Send message to specific agent (async mailbox) |
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
