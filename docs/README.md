# AgentMux

**MCP monitoring and inter-agent communication platform**

Version: 0.3.3 (Desktop) | 0.1.0 (CLI/MCP)
Status: 🚧 Active Development

---

## Overview

AgentMux enables Claude Code agents to communicate with each other across workspaces through multiple interfaces:

- **Desktop App** - Native GUI with embedded terminal, message bus, and agent monitoring
- **CLI** - Command-line interface for messaging (embedded in desktop app)
- **Wrapper** - Reactive PTY wrapper for supervised agent communication
- **MCP Server** - Model Context Protocol integration (experimental)

---

## Quick Start

### Installation

```bash
# From WebProjects root
cd agentmux

# Install dependencies
npm install

# Build all packages
npm run build

# Build desktop app
cd apps/desktop
npm run tauri:build
```

### Desktop App (Recommended)

The desktop app provides the full AgentMux experience:

```bash
# Run in development mode
cd apps/desktop
npm run tauri:dev

# Or run built binary
./src-tauri/target/release/agentmux-desktop
```

**Features:**
- Visual agent monitoring
- Embedded terminal with Claude Code
- Message bus interface
- Log export and viewing
- Reactive messaging

### CLI Commands (via Desktop App)

The desktop app includes a full CLI accessible from the embedded terminal:

```bash
# Agent management
agent list
agent info Agent1-*

# Messaging
bus send Agent1-* "Hello"
bus listen

# Logs
logs export --format json
```

See [apps/desktop/docs/CLI_STATUS.md](../apps/desktop/docs/CLI_STATUS.md) for complete command reference.

### Reactive Wrapper

Wrap AI CLIs for supervised inter-agent communication:

```bash
# Wrap Claude CLI
agentmux wrap claude --agent-id Agent3
```

See [apps/wrapper/docs/DEPLOYMENT.md](../apps/wrapper/docs/DEPLOYMENT.md) for setup guide.

---

## Documentation

### User Guides
- **[USAGE.md](USAGE.md)** - Complete usage guide
- **[INTEGRATION.md](INTEGRATION.md)** - Integration with your workflow
- **[QUICKSTART.md](QUICKSTART.md)** - Getting started in 5 minutes

### Developer Docs
- **[development/TESTING.md](development/TESTING.md)** - Testing guide
- **[development/TESTING_EMBEDDED_CLAUDE.md](development/TESTING_EMBEDDED_CLAUDE.md)** - Embedded terminal testing
- **[apps/desktop/docs/ARCHITECTURE.md](../apps/desktop/docs/ARCHITECTURE.md)** - Service layer architecture

### Deployment
- **[deployment/WRAPPER_DEPLOYMENT.md](deployment/WRAPPER_DEPLOYMENT.md)** - Wrapper deployment

### Specifications
- **[apps/desktop/docs/CLI_SPECIFICATION.md](../apps/desktop/docs/CLI_SPECIFICATION.md)** - CLI commands
- **[apps/desktop/docs/BUILD.md](../apps/desktop/docs/BUILD.md)** - Build instructions
- **[PROJECT_REORGANIZATION_SPEC.md](PROJECT_REORGANIZATION_SPEC.md)** - Project roadmap

### Archive
- **[archive/2025-10/](archive/2025-10/)** - Old test reports and notes

---

## Architecture

### Unified Structure

```
agentmux/
├── docs/                       # All documentation
│   ├── README.md              # This file
│   ├── development/           # Developer docs
│   ├── deployment/            # Deployment guides
│   └── archive/               # Historical docs
│
├── apps/
│   ├── desktop/               # Tauri desktop app (primary)
│   │   ├── src/              # SolidJS frontend
│   │   ├── src-tauri/        # Rust backend
│   │   │   ├── services/     # Business logic layer
│   │   │   ├── cli/          # CLI handlers
│   │   │   └── bus/          # Message bus
│   │   └── docs/             # Desktop-specific docs
│   │
│   ├── wrapper/               # PTY wrapper
│   ├── mcp-server/            # MCP protocol server
│   └── cli/                   # Standalone CLI (deprecated)
│
└── packages/
    └── core/                  # Shared core library
```

### Service Layer Architecture

AgentMux follows the **"One Operation, Three Interfaces"** pattern:

1. **Business Logic** - Lives in `services/` layer (Rust)
2. **CLI Interface** - Thin wrapper calling services
3. **Direct UI** - Tauri commands calling services

See [apps/desktop/docs/ARCHITECTURE.md](../apps/desktop/docs/ARCHITECTURE.md) for details.

---

## Key Features

### Desktop App v0.3.3
- ✅ Embedded Terminal - Full Claude Code integration
- ✅ CLI Commands - Agent, bus, and logs commands
- ✅ Log Export - Text and JSON formats
- ✅ Service Layer - Clean separation of concerns
- ✅ Reactive Messaging - Real-time notifications
- 🚧 Visual Monitoring - Agent dashboard (in progress)

### Wrapper v0.1.0
- ✅ PTY Integration - Full terminal emulation
- ✅ Reactive Notifications - <100ms latency
- ✅ Human Supervision - All messages visible

### CLI v0.1.0 (Standalone - Deprecated)
- ⚠️ Use desktop app's CLI instead

---

## Development

### Build System

```bash
# Build all
npm run build

# Development
npm run dev

# Tests
npm run test

# Lint
npm run lint
```

### Testing

```bash
# Desktop tests
cd apps/desktop
npm test

# Wrapper tests
cd apps/wrapper
npm test
```

See [development/TESTING.md](development/TESTING.md) for complete guide.

---

## Roadmap

### Phase 1: Documentation Organization (Current)
- ✅ Organize docs into `docs/` structure
- ✅ Archive old reports
- ✅ Update cross-references

### Phase 2: Unified Entry Point (Planned)
- [ ] Single `agentmux` command
- [ ] Intelligent routing (CLI vs Desktop)
- [ ] Unified user experience

### Phase 3: MCP Integration (Future)
- [ ] Evaluate MCP server
- [ ] Integrate with desktop app
- [ ] Claude Desktop integration

See [PROJECT_REORGANIZATION_SPEC.md](PROJECT_REORGANIZATION_SPEC.md) for complete roadmap.

---

## Contributing

See [development/](development/) for developer documentation.

Key principles:
- **Service layer first** - Business logic in `services/`
- **Test everything** - Unit tests required
- **Document changes** - Update docs with PRs
- **Follow architecture** - See ARCHITECTURE.md

---

## License

MIT

---

## Support

- **Issues:** https://github.com/a5af/agentmux/issues
- **Docs:** This repository's `docs/` folder
