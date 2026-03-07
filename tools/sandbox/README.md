# @agentmuxai/sandbox - AgentMux Development Tools

AgentMux development sandbox setup and management tools.

## Overview

This package provides automation scripts to configure a remote Windows machine as an isolated AgentMux development environment. This allows you to develop and test AgentMux without risking crashes on your main workstation.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    MAIN WORKSTATION                          │
│                                                              │
│  AgentMux (--instance=main)  ←──→  Parsec Client             │
│  Production agent interaction        Low-latency remote      │
└──────────────────────────────────────│───────────────────────┘
                                       │
                    Parsec P2P (4-8ms latency)
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────┐
│                    SANDBOX HOST                              │
│                                                              │
│  AgentMux (--instance=dev)   ←──→  Parsec Host               │
│  Development & testing              Always-connected VDA     │
│                                                              │
│  Tools: Node.js, Go, Zig, Task, Git, VS Code                │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

### On Sandbox Host (run once)

```powershell
# Full setup - installs everything
setup-sandbox-host

# Skip Parsec if already installed
setup-sandbox-host -SkipParsec

# Skip dev tools if already installed
setup-sandbox-host -SkipDevTools

# Clone specific AgentMux branch
setup-sandbox-host -AgentMuxBranch agentx/feature
```

### Health Check

```powershell
sandbox-health              # Quick check
sandbox-health -Verbose     # Detailed output
sandbox-health -OutputFormat json  # For automation
```

## What Gets Installed

### Development Tools

| Tool | Version | Purpose |
|------|---------|---------|
| Node.js | 18 LTS | Frontend build |
| Go | 1.21+ | Backend build |
| Zig | latest | Native compilation |
| Task | latest | Task runner |
| Git | latest | Version control |
| VS Code | latest | Editor |

### Remote Access

| Component | Purpose |
|-----------|---------|
| Parsec | Low-latency remote desktop (4-8ms) |
| Parsec VDA | Virtual display for headless operation |

### AgentMux

- Cloned to `D:\Code\sandbox\agentmux`
- Configured with `--instance=dev` for isolation
- Data stored in `~/.agentmux-dev/` (separate from production)

## Daily Workflow

```powershell
# 1. Connect to sandbox via Parsec from main workstation

# 2. Navigate to AgentMux
cd D:\Code\sandbox\agentmux

# 3. Start development server
task dev

# 4. Launch AgentMux with dev instance
agentmux --instance=dev

# 5. Make changes, test, repeat
# Frontend changes hot-reload automatically
# Backend changes require: task build:backend && restart task dev
```

## Scripts

### bin/setup-sandbox-host.ps1

Main entry point for sandbox setup. Wrapper that finds and executes the implementation script.

**Parameters:**
- `-SkipParsec` - Skip Parsec installation
- `-SkipDevTools` - Skip development tools installation
- `-Force` - Reinstall even if already present
- `-Verbose` - Enable detailed output
- `-AgentMuxBranch <branch>` - Clone specific branch (default: main)

### bin/sandbox-health.ps1

Health check wrapper for verifying sandbox configuration.

**Parameters:**
- `-Verbose` - Enable detailed output
- `-OutputFormat <text|json>` - Output format

## Configuration

### config/parsec-config.json

Parsec host configuration template for headless operation:

```json
{
  "host_virtual_monitor": 1,
  "host_virtual_monitor_fallback": 1,
  "host_privacy": 0
}
```

### config/agentmux-instance.json

AgentMux instance configuration:

```json
{
  "instance": "dev",
  "dataDir": "~/.agentmux-dev"
}
```

## Requirements

- Windows 10/11 Pro or Enterprise
- PowerShell Core (pwsh) 7.0+
- Internet access for downloads
- Administrator rights for some installations
- Parsec account (free tier works)

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Warnings (partial success) |
| 2 | Errors (failed) |
| 3 | Script failure |

## Troubleshooting

### Parsec won't connect without monitor

Enable virtual display in Parsec settings:
1. Settings → Host → "Fallback To Virtual Display" → ON
2. Or use HDMI dummy plug

### AgentMux crashes on startup

Check if correct instance is being used:
```powershell
# Should use --instance=dev
agentmux --instance=dev
```

### Build errors

Rebuild backend after Go changes:
```powershell
task build:backend
```

### Health check fails

Run verbose health check for details:
```powershell
sandbox-health -Verbose
```

## Development

**Location:** `agentmux/tools/sandbox`

**Structure:**
```
sandbox/
├── bin/
│   ├── setup-sandbox-host.ps1  # Setup wrapper
│   └── sandbox-health.ps1      # Health check wrapper
├── scripts/
│   ├── setup-sandbox-impl.ps1  # Main orchestrator
│   ├── install-dev-tools.ps1   # Tool installer
│   ├── install-parsec.ps1      # Parsec setup
│   ├── clone-agentmux.ps1       # AgentMux setup
│   └── sandbox-health-impl.ps1 # Health checks
├── config/
│   ├── parsec-config.json      # Parsec template
│   └── agentmux-instance.json   # Instance config
├── tests/
│   └── Sandbox.Tests.ps1       # Pester tests
├── package.json
└── README.md
```

## License

MIT
