# AgentMux Version History

This document tracks the version history of AgentMux (forked from waveterm).

## Latest Version: 0.31.18

**Base:** Upstream waveterm v0.12.0 + extensive custom features

---

## Version History (Latest First)

### v0.31.18-fork (2026-03-02)
- **Agent:** AgentA
- **Changes:**
  - feat: add Copy and Paste to pane right-click context menu
  - Copy reads xterm.js selection for terminals, window.getSelection() for others
  - Paste uses async clipboard read + terminal.paste() for terminal panes

### v0.31.17-fork (2026-03-02)
- **Agent:** AgentA
- **Changes:**
  - fix: correct package:portable:linux build — add missing copy:schema step, fix AppDir path, preserve AppImage output

### v0.31.16-fork (2026-03-01)
- **Agent:** AgentA
- **Changes:**
  - feat: pane right-click context menu — Split Up/Down/Left/Right + Open in VSCode

### v0.31.15-fork (2026-03-01)
- **Agent:** AgentA
- **Changes:**
  - chore: deploy build

### v0.31.14-fork (2026-03-01)
- **Agent:** AgentA
- **Changes:**
  - fix: restore shell integration scripts — deploy bash/zsh/pwsh/fish hooks on terminal start
  - fix: inject AGENTMUX_BLOCKID, AGENTMUX_TABID, TERM_PROGRAM env vars into PTY
  - fix: pane title and color from WAVEMUX_AGENT_ID now works again

### v0.31.13-fork (2026-02-28)
- **Agent:** AgentA
- **Changes:**
  - fix: agent Connect button — capture OAuth URL from PTY and open browser
  - feat: full-screen "Waiting for authorization" overlay during auth flow
  - docs: AGENT_AUTH_STATE_MACHINES.md — state machine reference

### v0.31.12-fork (2026-02-28)
- **Agent:** AgentA
- **Changes:**
  - feat: window instance indicator (1), (2) in title bar
  - docs: updated README, BUILD, CONTRIBUTING for 100% Rust stack

### v0.31.9-fork (2026-02-21)
- **Agent:** AgentA
- **Changes:**
  - perf: Convert Hack Nerd Mono fonts from TTF to WOFF2 (-5 MB)
  - perf: Exclude duplicate Monaco workers and NLS locales from static copy (-11.2 MB)
  - perf: Lazy-load WaveStreamdown to defer shiki (9.4 MB) to on-demand
  - perf: Strip redundant KaTeX TTF/WOFF fonts from build output (-876 KB)

### v0.31.4-fork (2026-02-21)
- **Agent:** AgentA
- **Changes:**
  - Simplified agent view to single Connect button flow
  - Removed debug logs and unused barrel exports from agent widget
  - Fixed #351: add copy:schema task to Taskfile.yml (dist/schema/ missing after clean)
  - Added screenshot patterns to .gitignore
  - Added debugging quick reference to CLAUDE.md

### v0.31.3-fork (2026-02-21)
- **Agent:** AgentA
- **Changes:**
  - Fix CLI auth flow: correct state machine, proper --verbose flag for stream-json
  - Remove hardcoded OAuth URL (was wrong endpoint)
  - Auth status check before session start

### v0.31.2-fork (2026-02-20)
- **Agent:** AgentA
- **Changes:**
  - Multi-provider CLI onboarding, auth management, and session abstraction
  - New providers/ directory with Claude, Codex, Gemini translator stubs
  - SetupWizard component for first-run onboarding
  - Rust backend providers.rs for multi-provider CLI auth checks
  - SPEC_CLAUDE_CLI_INTEGRATION.md design doc

### v0.31.0-fork (2026-02-20)
- **Agent:** AgentA
- **Changes:**
  - 100% Rust release: removed all Go source code (cmd/, pkg/, go.mod, go.sum)
  - wsh rewritten in Rust (wsh-rs crate): 1.1 MB binary vs 11 MB Go (90% size reduction)
  - Added sysinfo data collection to Rust backend (CPU, memory, network graphs)
  - Added getmeta, setmeta, waveinfo RPC handlers to Rust backend
  - Updated build system: all build tasks now use cargo (no Go/CGO dependency)
  - Binary size: agentmuxsrv-rs 4.4 MB + wsh 1.1 MB = 5.5 MB total (vs ~25 MB Go)

### v0.30.8-fork (2026-02-20)
- **Agent:** AgentA
- **Changes:**
  - Tree shake: delete 8 dead Rust modules (wcloud, shellutil, webhookdelivery, suggestion, telemetry, faviconcache, blocklogger, authkey)
  - Suppress 911 compiler warnings with #![allow(dead_code)] on Go-port modules
  - Remove all Electron references from frontend (rename ElectronApi → AppApi, ElectronContextMenuItem → NativeContextMenuItem, etc.)
  - Archive old docs/specs, reorganize debug scripts
  - Net removal of 3,449 lines of dead code

### v0.30.6-fork (2026-02-19)
- **Agent:** AgentA
- **Changes:**
  - Fix grey screen on startup: add 5s RPC timeouts and error recovery
  - showStartupError() renders user-facing error instead of blank screen
  - 30s safety-net timeout forces body visible if still hidden

### v0.30.5-fork (2026-02-19)
- **Agent:** AgentA
- **Changes:**
  - Modularize filestore.rs (1531 lines) into 7 focused files under filestore/ directory
  - No behavior changes — pure mechanical extraction
  - All 34 filestore tests pass

### v0.30.4-fork (2026-02-19)
- **Agent:** AgentA
- **Changes:**
  - Fix widgets, config event, and object CRUD in Rust backend

### v0.30.3-fork (2026-02-19)
- **Agent:** AgentA
- **Changes:**
  - Terminal I/O with real PTY support (portable-pty) in Rust backend
  - Wire controllerresync, controllerinput RPC handlers
  - Wire blockinput, setblocktermsize wscommands
  - Wire eventsub/eventunsub/eventunsuball to WPS Broker
  - Add EventBusBridge for Broker → EventBus → WebSocket event delivery
  - Replace unsafe run_lock pointer with safe Arc<AtomicBool>

### v0.30.0-fork (2026-02-17)
- **Agent:** AgentO
- **Changes:**
  - Rust backend parity fixes: match Go response shapes for all startup RPC calls
  - Fix meta null/empty serialization, otype in GetObject, isnew/pos/winsize defaults
  - Fix ListWorkspaces, GetAllConnStatus, tab naming, pinned tabs
  - Add parity test harness (scripts/parity-test.sh) — 8/8 tests pass
  - Default sidecar to Rust backend (agentmuxsrv-rs)

### v0.29.1-fork (2026-02-17)
- **Agent:** AgentX
- **Changes:**
  - Fix Linux AppImage build: use appimagetool when linuxdeploy crashes
  - Add agentmuxsrv-rs (Rust backend) to package:portable:linux build pipeline
  - Fix icon naming issue in AppDir (AgentMux.png → agentmux.png for desktop file)
  - Add scripts/build-appimage.sh with dynamic version and clear step ordering

### v0.29.0-fork (2026-02-16)
- **Agent:** AgentO
- **Changes:**
  - Wire Rust backend (agentmuxsrv-rs): replace all 501 stubs with real handlers
  - Implement full service dispatch (30+ methods: object, client, window, workspace, block, userinput)
  - Wire file endpoint, 9 reactive endpoints, WebSocket, AI chat SSE streaming, schema/docsite
  - Backend initialization: WaveStore, FileStore, EventBus, Broker, ReactiveHandler, Poller
  - Binary 9x smaller (3.1MB vs 28.5MB), memory 3.6x lower, latency 19-44% faster than Go
  - All 1089 unit tests + 4 integration tests pass

### v0.28.20-fork (2026-02-16)
- **Agent:** AgentO
- **Changes:**
  - Harden E2E tests: replace browser.pause() with proper waitUntil waits
  - Add data-testid attributes to UI components for stable test selectors
  - Create macOS-compatible WDIO config with mocked Tauri IPC
  - Add window-controls and layout regression test specs
  - Add byTestId() and waitForZoomChange() test helpers
  - Update SPEC_E2E_TESTING_MACOS.md with implementation details

### v0.28.5-fork (2026-02-15)
- **Agent:** AgentO
- **Changes:**
  - Remove notification bell icon from widget bar (unused dev-only UI)

### v0.28.4-fork (2026-02-15)
- **Agent:** AgentO
- **Changes:**
  - Fix: zsh "no matches found: wsh-*" error in shell integration
  - Use zsh (N) nullglob qualifier for portable wsh detection
  - Prevents zsh nomatch error when no wsh-* files exist in app directory

### v0.28.3-fork (2026-02-15)
- **Agent:** AgentO
- **Changes:**
  - Fix: Deploy wsh binary on macOS for shell integration
  - Set WAVETERM_APP_PATH env var so Go backend can locate wsh
  - Runtime copy of bundled wsh to bin/ with correct versioned name
  - Sync wsh binaries for dev mode in Taskfile.yml

### v0.27.14-fork (2026-02-15)
- **Agent:** AgentO
- **Changes:**
  - Fix: Skip systray on macOS to prevent backend crash (CGO signal fault in getlantern/systray)
  - Resolves blank screen issue on macOS ARM64

### v0.27.11-fork (2026-02-15)
- **Agent:** AgentX
- **Changes:**
  - Feat: Phase 5 - Unified Agent Widget Registration & Integration
  - Fix: Complete state scoping refactor - per-instance atoms to prevent state bleeding
  - Created AgentViewModel for state management and terminal streaming
  - Registered agent widget in block registry and widget config
  - Added AgentViewWrapper to bridge ViewModel and component interfaces
  - Enhanced stream parser with parseEvent() method
  - Users can now create and use unified agent widgets from UI
  - Completes Phases 1-5 of unified agent widget implementation

### v0.27.10-fork (2026-02-15)
- **Agent:** AgentX
- **Changes:**
  - Feat: Robust shell integration with self-healing
  - Add version guard to detect stale shell integration files
  - Implement multi-strategy wsh binary discovery (portable > installed > PATH)
  - Add defensive execution with graceful degradation
  - Wrap all wsh calls in Test-WshAvailable checks
  - Use -ErrorAction SilentlyContinue on all cleanup operations
  - Add template versioning support (AGENTMUX_VERSION, TIMESTAMP)

### v0.27.9-fork (2026-02-14)
- **Agent:** AgentX
- **Changes:**
  - Feat: Add `package:macos` task for platform-specific macOS builds
  - Creates .app and .dmg bundles on macOS
  - Documented CGO code signing limitations and workarounds
  - Fixed ExpectedVersion constant synchronization

### v0.26.0 (2026-02-12)
- **Agent:** AgentClaude
- **Changes:**
  - Feat: Display AgentMux version in tabbar (centered, clickable to copy)
  - Feat: Enable window dragging from entire tabbar area
  - Feat: Add right-click context menu to toggle widget visibility
  - Fix: Add macOS-specific version bump script (bump-version-osx.sh)

### v0.16.7 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Auto-load agentmux config from file on startup
  - Add LoadAgentMuxConfigFile() to load ~/.waveterm/agentmux.json
  - Add SaveAgentMuxConfigFile() to persist runtime config changes
  - ReconfigureGlobalPoller() now saves config to file automatically
  - No pre-configuration needed - just place agentmux.json and restart
  - Priority: config file < env vars (env vars override file config)

### v0.16.6 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Runtime agentmux config via wsh agentmux command
  - Add ReconfigureGlobalPoller() for runtime poller updates
  - Add HTTP endpoints: /wave/reactive/poller/config, /status
  - Add OSC 16162 "X" command for agentmux config
  - New wsh commands: `wsh agentmux config`, `wsh agentmux status`
  - Allows configuring AgentMux without restarting AgentMux

### v0.16.5 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Fix: Revert to synchronous Enter key for reactive injection
  - Add rate limiter (10 req/sec) for DoS protection
  - Docs: Add REACTIVE_INJECTION_REGRESSION_REPORT.md

### v0.16.4 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Fix: Enter key retry with 3 attempts (still broken)
  - Added documentation for the issue

### v0.16.3 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Fix: Enter key timing for reactive injection (300ms delay, CRLF)
  - Added retry after 700ms (still broken)

### v0.16.2 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Made Enter key async to prevent DoS (breaking change)
  - This change broke message/Enter coordination

### v0.16.1 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Cross-host reactive messaging poller (#144)
  - AgentMux polls AgentMux for pending injections from remote agents
  - New endpoint: /wave/reactive/poller/stats for monitoring
  - Configurable via AGENTMUX_URL, AGENTMUX_TOKEN env vars
  - Enables agent-to-agent messaging across different machines

### v0.16.0 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Reactive agent-to-agent messaging (#140, #141)
  - Inject messages directly into running Claude Code instances
  - New HTTP API: /wave/reactive/inject, /agents, /register, /unregister, /audit
  - Frontend auto-registers agents via OSC 16162 WAVEMUX_AGENT_ID
  - Message sanitization and audit logging
  - AgentMux inject_terminal MCP tool (agentmux#69)

### v0.15.15 (2026-01-16)
- **Agent:** AgentX
- **Changes:**
  - Feat: add WAVEMUX_AGENT_TEXT_COLOR support for pane header text (#137)
  - Customizable text color for agent pane headers
  - Smart defaults: white text on dark backgrounds, black on light

### v0.15.14 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Refactor: remove AGENTMUX_AGENT_ID coupling (#135)
  - AgentMux now only uses WAVEMUX_AGENT_ID for agent identity
  - Shell integration scripts cleaned of AGENTMUX fallbacks

### v0.15.13 (2026-01-15)
- **Agent:** AgentA
- **Changes:**
  - Fix: prevent duplicate title display in pane header and titlebar (#134)
  - Fix: decouple system hostname from agent detection (#132)

### v0.15.12 (2026-01-15)
- **Agent:** AgentA
- **Changes:**
  - Docs: update VERSION_HISTORY.md to reflect current state (#130)

### v0.15.9 - v0.15.11 (2026-01-15)
- **Agent:** AgentA
- **Changes:**
  - Fix: disable hostname-based agent detection for local terminals (#127)
  - Local terminals no longer auto-detect agent from hostname patterns
  - Explicit `agent-workspaces` directory pattern still works
  - Env vars (WAVEMUX_AGENT_ID) take highest priority

### v0.15.5 - v0.15.8 (2026-01-14)
- **Agent:** AgentX
- **Changes:**
  - Fix: Claude activity display - no duplicate, bold in header (#126)
  - Fix: per-pane agent identification + build system fixes (#125)
  - Fix: re-enable hardware acceleration by default (#123)

### v0.15.4 (2026-01-13)
- **Agent:** AgentX
- **Changes:**
  - Feat: add AgentY to default agent colors (#122)
  - Feat: Display Claude activity summaries in pane title bar (#121)
  - Feat: per-pane agent colors via shell environment variables (#120)
  - Fix: improve agent detection path matching with trailing slash (#119)

### v0.15.0 - v0.15.3 (2026-01-12)
- **Agent:** AgentX
- **Changes:**
  - Feat: Add agent colors to terminal pane headers (#103)
  - Feat: environment variable-based agent detection (#102)
  - Disable Dependabot - causing too many blockers (#118)
  - Sync missing aiprompts files from upstream waveterm

### v0.14.0 (2026-01-09)
- **Changes:**
  - Removed Storybook (unused dev tool, ~36MB savings)
  - Removed Storybook references from Dependabot config
  - Fixed remote desktop startup failures (reverted to simple 1-terminal layout)
  - Disabled hardware acceleration for Windows Sandbox/RDP compatibility
  - Added console window with verbose startup logging
  - Multiple dependency updates (xterm, monaco, react-hook-form, etc.)

### v0.13.3 - v0.13.6 (2026-01-08)
- **Changes:**
  - Various hardware acceleration and startup fixes
  - Window size calculation debugging
  - Layout fixes for remote desktop

### v0.12.14-fork (2025-10-20)

- **Branch:** `feature/high-contrast-terminal-borders`
- **Agent:** agentx
- **Changes:**
  - **P0 FIX:** Cross-platform wsh binary exclusions (breaks macOS/Linux builds)
  - **P1 FIX:** Updater IPC handler crash when auto-update disabled
  - Added RELEASE_CHECKLIST.md with comprehensive workflow guide
  - Enhanced bump-version.sh to prevent releasing old code
  - Documented correct release workflow to prevent v0.12.13 issue recurrence

### v0.12.13-fork (2025-10-20)

- **Branch:** `feature/high-contrast-terminal-borders`
- **Agent:** agentx
- **Changes:**
  - Fix title bar instance number parsing bug (was showing "undefined")
  - Add comprehensive app name and instance tests
  - **NOTE:** This version was released BEFORE instance parsing fix was committed
  - **ISSUE:** Users downloaded old code under new version number
  - **RESOLUTION:** v0.12.14 includes all fixes with corrected workflow

### v0.12.12-fork (2025-10-20)

- **Branch:** `feature/high-contrast-terminal-borders`
- **Changes:**
  - Package verification and version consistency fixes

### v0.12.11-fork (2025-10-20)
- **Changes:**
  - Version management improvements and documentation

### v0.12.10-fork (2025-10-19)

- **Branch:** `feature/high-contrast-terminal-borders`
- **Changes:**
  - Fix waveConfigDirName undefined error
  - Add smoke tests for configuration

### v0.12.9-fork (2025-10-19)
- **Changes:**
  - Fix waveDirName undefined error
  - Add more configuration tests

### v0.12.8-fork (2025-10-19)
- **Changes:**
  - Implement portable multi-instance mode with persistent settings

### v0.12.7-fork (2025-10-19)
- **Changes:**
  - UI improvements: hard corners, better borders
  - Fix settings persistence issues
  - Optimize build size: remove heavy artifacts

### v0.12.6-fork (2025-10-19)
- **Changes:**
  - Add comprehensive crash reporting system

### v0.12.3-fork (2025-10-19)

- **Agent:** agentx
- **Branch:** `agentx/merge-upstream-v0.12.0`
- **Changes:**
  - Add high-contrast white borders to unselected terminal blocks
  - Fix electron-builder packaging bug (upgrade to v26.1.0)
  - Document critical electron-builder files configuration bug
  - Add build investigation spec and artifact verification
  - Added multi-instance development support
  - Added comprehensive documentation (BUILD.md, CLAUDE.md)
  - **Added version management scripts (bump-version.sh/ps1) and this VERSION_HISTORY.md**

### v0.12.2-fork
- Multi-instance support improvements
- Multi-instance dialog

### v0.12.1-fork
- Inherit main install profile
- Initial multi-instance support with shared config

### v0.12.0-fork
- Initial merge from upstream v0.12.0

---

## Development Setup

### Branch Naming

Feature branches follow the pattern: `feature/description` or `agent/feature-name`.

---

## Upstream Version Tracking

- **Upstream repository:** https://github.com/wavetermdev/waveterm
- **Base Upstream Version:** v0.12.0
- **Fork repository:** https://github.com/agentmuxhq/agentmux
- **Latest Fork:** v0.31.4
- **Commits Ahead of Upstream:** 100+ commits with custom features

---

## Key Fork Features

1. **Per-pane agent identification** - Terminal panes show agent identity (AgentA, AgentX, etc.)
2. **Agent color borders** - Colored borders indicate which agent owns a pane
3. **Claude activity display** - Shows Claude Code activity summaries in title bar
4. **Environment-based agent detection** - WAVEMUX_AGENT_ID env var
5. **OSC 16162 shell integration** - Shell can send agent identity via escape sequences
6. **Multi-instance support** - Multiple AgentMux instances can run simultaneously
7. **Portable mode** - Persistent settings across instances
8. **High-contrast borders** - Visual improvements for terminal blocks
9. **Version management** - Automated version bump scripts

---

## Version Bump Instructions

```bash
# Bump patch version (0.15.8 -> 0.15.9)
./bump-version.sh patch --message "Fix description"

# Bump minor version (0.15.8 -> 0.16.0)
./bump-version.sh minor --message "New feature"
```

The bump scripts automatically:
- ✅ Update `package.json` and `package-lock.json`
- ✅ Create git commit with version message
- ✅ Create git tag (e.g., `v0.15.9-fork`)

---

## Notes for Agents

- Always check this file first to understand current version state
- Create feature branches from `main`: `git checkout -b agentX/feature-name`
- Open PRs against `main` branch (it's protected, requires PR)
- Run `task build:backend` after Go changes
- Run `task dev` for development with hot reload
- Run `task package` only for final release builds
