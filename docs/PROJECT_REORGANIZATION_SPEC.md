# AgentMux Project Reorganization Specification

**Version:** 1.0
**Date:** 2025-10-13
**Status:** Proposed

---

## Executive Summary

AgentMux currently has 4 separate apps (cli, desktop, mcp-server, wrapper) with scattered documentation and unclear entry points. This spec consolidates the project into a unified `agentmux` command that intelligently routes to either CLI or Desktop app, organizes all documentation, and removes unused code.

---

## Current State Analysis

### Apps Structure

| App | Status | Purpose | Usage |
|-----|--------|---------|-------|
| **cli** | ✅ Active | Message bus CLI (`agentmux send/listen`) | File-based agent messaging |
| **desktop** | ✅ Active | Tauri desktop app with GUI + embedded CLI | Visual monitoring, built-in terminal |
| **wrapper** | ✅ Active | PTY wrapper for reactive notifications | Wraps AI CLIs with message alerts |
| **mcp-server** | ❓ Unclear | MCP protocol server | Unknown usage, no documentation |

### Documentation Issues

**Root-level docs (unorganized):**
- `README.md` - Project overview
- `USAGE.md` - CLI usage
- `INTEGRATION.md` - Integration guide
- `WRAPPER_DEPLOYMENT_GUIDE.md` - Wrapper deployment

**Desktop app docs (scattered in `apps/desktop/`):**
- `README.md`, `QUICKSTART.md` - Duplicates root docs
- `CLI_SPECIFICATION.md`, `BUILD-STATUS.md` - Specs
- `TESTING.md`, `TESTING_EMBEDDED_CLAUDE.md` - Testing
- `TEST_*.md`, `FINAL_TEST_REPORT.md` - Old test reports
- `SPEC_*.md` - Multiple spec docs
- `IMPLEMENTATION_SUMMARY.md` - Implementation notes
- `REACTIVE-DEMO-GUIDE.md` - Demo guide
- `FRONTEND_TEST_COVERAGE.md` - Coverage report

**Existing organized docs:**
- `docs/` - Has `MCP_SERVER_SETUP.md`, `QUICKSTART.md`
- `apps/desktop/docs/` - Has `ARCHITECTURE.md`, `CLI_STATUS.md`

### Entry Point Confusion

Users must know whether to run:
- `agentmux` (CLI) - For terminal usage
- `agentmux-desktop` (Tauri app) - For GUI
- `agentmux-wrap` (Wrapper) - For reactive wrapping
- `agentmux-mcp` (MCP server) - For MCP protocol

**Problem:** No unified entry point, users must choose the right binary.

---

## Proposed Architecture

### Unified Entry Point: `agentmux`

Single command that intelligently routes based on context:

```bash
# CLI mode (default when no GUI available or --cli flag)
agentmux send Agent1-* "message"
agentmux listen
agentmux wrap claude

# Desktop mode (launches GUI)
agentmux --gui
agentmux desktop

# Explicit modes
agentmux --cli send Agent1-* "message"
agentmux --desktop
```

**Implementation:**
1. Main launcher script detects environment (GUI available? TTY?)
2. Routes to appropriate app based on:
   - Command flags (`--gui`, `--cli`, `--desktop`)
   - Available display (DISPLAY env var, Windows GUI session)
   - Command structure (if command is `send`/`listen`/`wrap` → CLI mode)
3. Desktop app includes all CLI functionality via embedded terminal

### App Consolidation

```
apps/
├── desktop/              # Tauri app (includes GUI + CLI functionality)
│   ├── src/             # SolidJS frontend
│   ├── src-tauri/       # Rust backend
│   │   ├── src/
│   │   │   ├── services/    # Business logic layer
│   │   │   ├── cli/         # CLI handlers
│   │   │   ├── bus/         # Message bus
│   │   │   └── embedded_claude/ # Embedded terminal
│   │   └── Cargo.toml
│   └── docs/            # Desktop-specific docs
│       ├── ARCHITECTURE.md
│       ├── CLI_STATUS.md
│       └── BUILD.md
│
├── wrapper/             # PTY wrapper for reactive notifications
│   ├── src/
│   └── docs/
│       └── DEPLOYMENT.md
│
└── mcp-server/          # MCP protocol server (if still needed)
    ├── src/
    └── docs/
        └── MCP_SETUP.md
```

**Deprecate:** `apps/cli/` - Functionality moved to desktop app's CLI module

### Documentation Organization

```
docs/                         # Root documentation
├── README.md                 # Project overview (move from root)
├── QUICKSTART.md            # Getting started
├── ARCHITECTURE.md          # High-level architecture
├── USAGE.md                 # User guide
├── INTEGRATION.md           # Integration guide
│
├── development/             # Developer docs
│   ├── CONTRIBUTING.md
│   ├── TESTING.md
│   └── BUILD.md
│
├── deployment/              # Deployment docs
│   ├── WRAPPER_DEPLOYMENT.md
│   └── PRODUCTION.md
│
└── archive/                 # Old docs (for reference)
    ├── 2025-10/
    │   ├── OLD_CLI_README.md
    │   ├── TEST_REPORTS.md
    │   └── IMPLEMENTATION_NOTES.md
    └── .gitkeep

apps/desktop/docs/           # Desktop-specific docs
├── ARCHITECTURE.md          # Service layer architecture
├── CLI_STATUS.md            # CLI implementation status
└── BUILD.md                 # Build instructions

apps/wrapper/docs/           # Wrapper-specific docs
└── DEPLOYMENT.md            # Wrapper deployment guide

apps/mcp-server/docs/        # MCP-specific docs (if kept)
└── MCP_SETUP.md             # MCP server setup
```

---

## Migration Plan

### Phase 1: Documentation Organization (This PR)

**Tasks:**
1. Create `docs/` structure with subdirectories
2. Move root-level docs:
   - `README.md` → `docs/README.md` (keep symlink in root)
   - `USAGE.md` → `docs/USAGE.md`
   - `INTEGRATION.md` → `docs/INTEGRATION.md`
   - `WRAPPER_DEPLOYMENT_GUIDE.md` → `docs/deployment/WRAPPER_DEPLOYMENT.md`

3. Consolidate `apps/desktop/` docs:
   - Keep `docs/ARCHITECTURE.md`, `docs/CLI_STATUS.md`
   - Move testing docs → `docs/development/TESTING.md`
   - Archive old reports → `docs/archive/2025-10/`
   - Remove duplicates

4. Update all internal doc references

**Files to archive:**
- `apps/desktop/FINAL_TEST_REPORT.md`
- `apps/desktop/FRONTEND_TEST_COVERAGE.md`
- `apps/desktop/TEST_AGENT_CONNECTION.md`
- `apps/desktop/TEST_COVERAGE_REPORT.md`
- `apps/desktop/IMPLEMENTATION_SUMMARY.md`
- `apps/desktop/REACTIVE-DEMO-GUIDE.md`

**Files to consolidate:**
- Merge `apps/desktop/TESTING.md` + `TESTING_EMBEDDED_CLAUDE.md` → `docs/development/TESTING.md`
- Merge `apps/desktop/QUICKSTART.md` into `docs/QUICKSTART.md`

### Phase 2: Unified Entry Point (Future PR)

**Tasks:**
1. Create main launcher script `agentmux` (Node.js/Shell)
2. Implement routing logic:
   ```typescript
   if (args.includes('--gui') || args.includes('--desktop')) {
     launchDesktopApp();
   } else if (hasDisplay() && !args.includes('--cli')) {
     offerGUI();
   } else {
     runCLI();
   }
   ```
3. Update package.json bins to use launcher
4. Update all documentation with new usage patterns

### Phase 3: MCP Server Evaluation (Future PR)

**Tasks:**
1. Audit MCP server usage
2. If unused → deprecate and archive
3. If used → document clearly and integrate with desktop app
4. Update architecture diagrams

---

## Implementation Checklist

### This PR: Documentation Organization

- [ ] Create `docs/` directory structure
- [ ] Create `docs/development/` subdirectory
- [ ] Create `docs/deployment/` subdirectory
- [ ] Create `docs/archive/2025-10/` subdirectory
- [ ] Move `README.md` → `docs/README.md` (symlink in root)
- [ ] Move `USAGE.md` → `docs/USAGE.md`
- [ ] Move `INTEGRATION.md` → `docs/INTEGRATION.md`
- [ ] Move `WRAPPER_DEPLOYMENT_GUIDE.md` → `docs/deployment/WRAPPER_DEPLOYMENT.md`
- [ ] Consolidate testing docs → `docs/development/TESTING.md`
- [ ] Archive old test reports to `docs/archive/2025-10/`
- [ ] Move specs to `apps/desktop/docs/`
- [ ] Remove duplicate README/QUICKSTART from `apps/desktop/`
- [ ] Update all cross-references in docs
- [ ] Verify builds still work
- [ ] Update root README with new doc structure

---

## Benefits

### For Users
- **Single command:** No confusion about which binary to run
- **Clear documentation:** Organized by purpose (user guide vs developer vs deployment)
- **Better onboarding:** Consolidated quickstart and usage guides

### For Developers
- **Less duplication:** Single source of truth for each concept
- **Clear architecture:** Service layer documented and enforced
- **Easy testing:** All docs in predictable locations
- **Better maintenance:** Archived old docs prevent confusion

### For Project
- **Professional appearance:** Clean, organized structure
- **Easier contributions:** Clear where to add new docs
- **Better discovery:** Related docs grouped together
- **Reduced clutter:** Archive old implementation notes

---

## Breaking Changes

### Phase 1 (This PR): None
- All moves maintain backward compatibility
- Symlinks preserve root-level README access

### Phase 2 (Future): Minor
- Users may need to update scripts using `apps/cli/dist/index.js` directly
- New `agentmux` command may conflict with globally linked old CLI

### Phase 3 (Future): Potentially Major
- If MCP server is deprecated, users relying on it will need migration path

---

## Success Metrics

1. **Documentation findability:** <30 seconds to find any doc via README
2. **Reduced duplication:** <10% duplicate content across docs
3. **Clear entry point:** 100% of new users know which command to run
4. **Archive hygiene:** 0 files in root from >1 month ago

---

## Questions to Resolve

1. **MCP Server:** Is it actively used? Can we deprecate?
2. **CLI App:** Can we fully deprecate `apps/cli/` in favor of desktop's CLI module?
3. **Wrapper Integration:** Should wrapper be part of desktop app or stay separate?

---

## Next Steps

1. Review and approve this spec
2. Implement Phase 1 (documentation organization)
3. Create PR with all doc moves and archives
4. Plan Phase 2 implementation after Phase 1 merges
