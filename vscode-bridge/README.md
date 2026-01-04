# VS Code Bridge

**Host-side service for opening files in VS Code from container agents**

## Overview

Container agents can't directly run Windows commands. This bridge service runs on the Windows host and receives HTTP requests from agentmux, translating container paths to host paths and opening files in VS Code.

```
Container (agentmux)  ──HTTP──►  Host (vscode-bridge:3101)  ──►  code --goto file
```

## Installation

```powershell
# Clone or navigate to agentmux/vscode-bridge
cd ~/.claw/agentmux/vscode-bridge

# Install as Windows service (auto-start on login, auto-restart on failure)
.\install-service.ps1

# Or run manually
node index.js
```

## Endpoints

### GET /health

Health check for verifying the bridge is running.

```bash
curl http://localhost:3101/health
```

Response:
```json
{
  "status": "ok",
  "service": "vscode-bridge",
  "version": "1.0.0",
  "timestamp": "2026-01-01T12:00:00.000Z",
  "workspacesBase": "C:\\Users\\asafe\\.claw\\workspaces"
}
```

### POST /open

Open a file in VS Code.

```bash
curl -X POST http://localhost:3101/open \
  -H "Content-Type: application/json" \
  -d '{"agentId": "agent2", "path": "/workspace/src/index.ts", "line": 42}'
```

Parameters:
- `agentId` (required): Agent ID for path translation (e.g., "agent2")
- `path` (required): File path (container path like `/workspace/src/file.ts`)
- `line` (optional): Line number to navigate to
- `column` (optional): Column number to navigate to

Response:
```json
{
  "success": true,
  "path": "C:\\Users\\asafe\\.claw\\workspaces\\agent2\\src\\index.ts",
  "line": 42
}
```

## Path Translation

Container paths are translated to host paths:

| Container Path | Host Path |
|----------------|-----------|
| `/workspace/src/file.ts` | `~/.claw/workspaces/{agentId}/src/file.ts` |

## Configuration

Environment variables:
- `VSCODE_BRIDGE_PORT` - Port to listen on (default: 3101)
- `VSCODE_BRIDGE_HOST` - Host to bind to (default: 0.0.0.0)
- `CLAW_WORKSPACES_DIR` - Base directory for workspaces (default: ~/.claw/workspaces)

## Uninstall

```powershell
.\install-service.ps1 -Uninstall
```

## Usage from Container Agents

From agentmux MCP tool:
```
mcp__agentmux__open_vscode({ path: "/workspace/src/index.ts", line: 42 })
```

The agentmux server proxies this to `host.docker.internal:3101`.
