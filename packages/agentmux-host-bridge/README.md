# @a5af/agentmux-host-bridge

MCP server that allows container agents to execute allowed commands on the Windows host.

## Features

- MCP server exposing host command execution
- Strict command allowlist for security
- 30-second timeout per command
- Runs as Windows service via NSSM

## Security

Only these commands are allowed:
- `vscode` - Visual Studio Code
- `explorer` - Windows Explorer
- `git` - Git CLI
- `notepad` - Notepad
- `terminal` - Windows Terminal (wt)
- `powershell` - PowerShell
- `cmd` - Command Prompt

All other commands are rejected.

## Installation

```bash
npm install @a5af/agentmux-host-bridge
```

## Usage

### As MCP Server

Add to `.mcp.json`:

```json
{
  "mcpServers": {
    "host-bridge": {
      "command": "npx",
      "args": ["@a5af/agentmux-host-bridge"]
    }
  }
}
```

### MCP Tools

#### execute_command

Execute an allowed command on the host.

**Parameters:**
- `command` (string, required): Command to execute (from allowed list)
- `args` (string[], optional): Arguments to pass to the command
- `cwd` (string, optional): Working directory

**Example:**
```typescript
{
  "command": "vscode",
  "args": ["C:\\Users\\asafe\\.claw\\agentx-workspace"],
  "cwd": "C:\\Users\\asafe"
}
```

**Response:**
```json
{
  "success": true,
  "stdout": "",
  "stderr": ""
}
```

#### list_allowed_commands

Get list of commands that can be executed.

**Returns:**
```json
{
  "commands": ["vscode", "explorer", "git", "notepad", "terminal", "powershell", "cmd"]
}
```

## Running as Windows Service

### Using NSSM

1. Install NSSM:
```powershell
choco install nssm
```

2. Install service:
```powershell
nssm install AgentMuxHostBridge "C:\Program Files\nodejs\node.exe" "C:\Users\asafe\.claw\agentx-workspace\agentmux\packages\agentmux-host-bridge\dist\index.js"
```

3. Configure service:
```powershell
nssm set AgentMuxHostBridge AppDirectory "C:\Users\asafe\.claw\agentx-workspace\agentmux\packages\agentmux-host-bridge"
nssm set AgentMuxHostBridge DisplayName "AgentMux Host Bridge"
nssm set AgentMuxHostBridge Description "MCP server for host command execution"
nssm set AgentMuxHostBridge Start SERVICE_AUTO_START
```

4. Start service:
```powershell
nssm start AgentMuxHostBridge
```

### Service Management

```powershell
# Check status
nssm status AgentMuxHostBridge

# Stop service
nssm stop AgentMuxHostBridge

# Restart service
nssm restart AgentMuxHostBridge

# Remove service
nssm remove AgentMuxHostBridge confirm
```

## Development

```bash
# Build
npm run build

# Run locally
npm start

# Build and run
npm run dev
```

## Security Notes

- Commands are executed with the permissions of the service account
- 30-second timeout prevents long-running commands
- 1MB output buffer limit
- No arbitrary command execution - strict allowlist only
- Consider running service with limited user permissions

## License

MIT
