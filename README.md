# AgentMux

**Native desktop app for agent monitoring and inter-agent communication**

Version: 0.3.20
Status: 🚧 Active Development

---

## Overview

AgentMux is a native desktop application that enables Claude Code agents to communicate with each other across workspaces:

- **Embedded Terminal** - Full terminal with Claude Code integration
- **Message Bus** - Real-time inter-agent messaging
- **Agent Monitoring** - Visual dashboard for agent status
- **Built-in CLI** - Agent and bus commands accessible from terminal

---

## Quick Start

### Installation

```bash
# Clone or navigate to agentmux
cd agentmux

# Install dependencies
npm install

# Build desktop app
npm run tauri:build
```

### Running

```bash
# Development mode
npm run tauri:dev

# Or run built binary
./src-tauri/target/release/agentmux
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

See [docs/CLI_STATUS.md](docs/CLI_STATUS.md) for complete command reference.

---

## Documentation

### User Guides
- **[USAGE.md](USAGE.md)** - Complete usage guide
- **[INTEGRATION.md](INTEGRATION.md)** - Integration with your workflow
- **[QUICKSTART.md](QUICKSTART.md)** - Getting started in 5 minutes

### Developer Docs
- **[docs/development/TESTING.md](docs/development/TESTING.md)** - Testing guide
- **[docs/development/TESTING_EMBEDDED_CLAUDE.md](docs/development/TESTING_EMBEDDED_CLAUDE.md)** - Embedded terminal testing
- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** - Service layer architecture

### Specifications
- **[docs/CLI_SPECIFICATION.md](docs/CLI_SPECIFICATION.md)** - CLI commands
- **[docs/BUILD.md](docs/BUILD.md)** - Build instructions

### Archive
- **[archive/2025-10/](archive/2025-10/)** - Old test reports and notes

---

## Architecture

### Project Structure

```
agentmux/
├── src/                       # SolidJS frontend
├── src-tauri/                 # Rust backend
│   ├── src/
│   │   ├── services/         # Business logic layer
│   │   ├── cli/              # CLI handlers
│   │   └── bus/              # Message bus
│   └── Cargo.toml
├── tests/                     # Test suites
├── docs/                      # Documentation
│   ├── development/          # Developer guides
│   └── archive/              # Historical docs
└── package.json              # Desktop app configuration
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

### Version Management

**⚠️ IMPORTANT: Every build MUST increment the version number**

Before running `npm run tauri:build`, update both version files to avoid duplicate releases:

1. Update `apps/desktop/package.json`:
   ```bash
   cd apps/desktop
   npm version patch  # or minor, major
   ```

2. Update `apps/desktop/src-tauri/Cargo.toml` to match:
   ```toml
   version = "0.3.0"  # Must match package.json
   ```

3. Update `apps/desktop/src-tauri/tauri.conf.json` to match:
   ```json
   "version": "0.3.0"  // Must match package.json - CRITICAL for MSI filename
   ```

4. Then build:
   ```bash
   npm run tauri:build
   ```

**Why this matters:**
- Multiple builds with the same version (e.g., 0.2.9) create duplicate MSI files
- Version mismatches between package.json and Cargo.toml cause confusion
- MSI installers are named by version: `AgentMux Desktop_{version}_x64_en-US.msi`

**Current version:** Check `apps/desktop/package.json` for the latest version number.
