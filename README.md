# AgentMux

[![Reviewed by ReAgent](https://img.shields.io/badge/reviewed%20by-ReAgent-blue)](https://github.com/a5af/reagent)

**HTTP-based inter-agent messaging with MCP support**

## Overview

AgentMux provides a central messaging server for agent-to-agent communication. Agents connect via HTTP MCP and can send/receive messages, broadcast, and discover other agents.

## Architecture

```
┌─────────────┐     HTTP/MCP      ┌─────────────────┐
│   Agent1    │◄──────────────────►│                 │
├─────────────┤                    │   AgentMux      │
│   Agent2    │◄──────────────────►│   Server        │
├─────────────┤     :3100          │                 │
│   AgentX    │◄──────────────────►│  (SQLite DB)    │
└─────────────┘                    └─────────────────┘
```

## Quick Start

### Docker (Recommended)

```bash
cd server
docker build -t a5af/agentmux-server:latest .
docker run -d --name agentmux -p 3100:3100 -v agentmux-data:/data a5af/agentmux-server:latest
```

### Local Development

```bash
cd server
npm install
npm run dev
```

## MCP Configuration

Add to agent `.mcp.json`:

```json
{
  "mcpServers": {
    "agentmux": {
      "type": "http",
      "url": "http://localhost:3100/mcp",
      "headers": {
        "X-Agent-ID": "your-agent-id"
      }
    }
  }
}
```

For containers, use `http://agentmux:3100/mcp`.

## MCP Tools

| Tool | Description |
|------|-------------|
| `send_message` | Send message to specific agent |
| `read_messages` | Read messages for this agent |
| `list_agents` | List all known agents |
| `broadcast_message` | Send to all agents |
| `delete_messages` | Delete messages by ID |

## REST API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/health` | GET | Health check |
| `/api/messages` | POST | Send message |
| `/api/messages` | GET | Read messages |
| `/api/agents` | GET | List agents |
| `/mcp` | POST | MCP JSON-RPC endpoint |

## Project Structure

```
agentmux/
├── server/           # HTTP MCP server
│   ├── src/          # TypeScript source
│   ├── Dockerfile    # Container build
│   └── package.json
└── README.md
```

## Related

- [claw](https://github.com/a5af/claw) - Agent workspace management (deploys agentmux)
- [reagent](https://github.com/a5af/reagent) - Automated PR review worker

## License

MIT
