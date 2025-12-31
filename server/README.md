# AgentMux HTTP Server

**HTTP-based inter-agent messaging server with MCP support**

Version: 1.0.0

---

## Overview

Central messaging server for agent communication. Replaces the deprecated file-based MCP server with a robust HTTP architecture.

### Features

- HTTP MCP transport (JSON-RPC over POST)
- REST API for direct integration
- SQLite persistence
- Cross-host ready (federation planned)
- No shared volumes required

---

## Quick Start

### Docker (Recommended)

```bash
# Build
docker build -t a5af/agentmux-server:latest .

# Run
docker run -d --name agentmux \
  -p 3100:3100 \
  -v agentmux-data:/data \
  a5af/agentmux-server:latest
```

### Local Development

```bash
npm install
npm run dev
```

---

## API Endpoints

### Health Check
```
GET /api/health
```

### Send Message
```
POST /api/messages
Headers: X-Agent-ID: agent1
Body: { "to": "agent2", "message": "Hello", "priority": "normal" }
```

### Read Messages
```
GET /api/messages?unread_only=true&limit=10
Headers: X-Agent-ID: agent1
```

### List Agents
```
GET /api/agents
```

### MCP (JSON-RPC)
```
POST /mcp
Headers: X-Agent-ID: agent1
Body: { "jsonrpc": "2.0", "id": 1, "method": "tools/list" }
```

---

## MCP Configuration

Add to agent `.mcp.json`:

```json
{
  "mcpServers": {
    "agentmux": {
      "type": "http",
      "url": "http://agentmux:3100/mcp",
      "headers": {
        "X-Agent-ID": "agent1"
      }
    }
  }
}
```

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| PORT | 3100 | Server port |
| HOST | 0.0.0.0 | Bind address |
| DB_PATH | /data/agentmux.db | SQLite database path |

---

## Docker Compose

```yaml
services:
  agentmux:
    image: a5af/agentmux-server:latest
    container_name: agentmux
    hostname: agentmux
    ports:
      - "3100:3100"
    volumes:
      - agentmux-data:/data
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://localhost:3100/api/health"]
      interval: 30s
    restart: unless-stopped

volumes:
  agentmux-data:
```

---

## Migration from File-Based

1. Deploy this server
2. Update agent MCP configs to use HTTP
3. Remove `~/.agentmux` volume mounts
4. Delete old `@a5af/agentmux-mcp` package references

---

## License

MIT
