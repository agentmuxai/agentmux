# AgentMux Desktop Releases

## v0.3.0 (2025-10-14)

**Key Features:**
- ✅ Fixed WebSocket stdin forwarding - UI messages now reach Claude CLI
- ✅ Embedded terminal with full Claude Code integration
- ✅ Agent spawning and management via UI
- ✅ Message bus interface
- ✅ Log export (text/JSON)

**Downloads:**
- `agentmux-desktop-v0.3.0-portable.exe` (19M) - Portable executable, no installation required
- `agentmux-desktop-v0.3.0-installer.msi` (6.2M) - Windows installer package

**Installation:**

**Option 1: Portable (Recommended for testing)**
1. Download `agentmux-desktop-v0.3.0-portable.exe`
2. Run directly - no installation needed

**Option 2: MSI Installer**
1. Download `agentmux-desktop-v0.3.0-installer.msi`
2. Double-click to install
3. Find "AgentMux Desktop" in Start Menu

**Requirements:**
- Windows 10/11 (x64)
- WebView2 runtime (auto-installed by MSI)

**Usage:**
1. Launch AgentMux Desktop
2. Click "Spawn Agent" to create a Claude instance
3. Type messages in the terminal input
4. Messages are sent to Claude via WebSocket → stdin

---

## Version History

### v0.3.0 (2025-10-14)
- Fixed WebSocket stdin forwarding
- Added version management documentation
- Organized releases folder

### v0.2.9 (2025-10-13)
- Multiple builds (duplicates) - fixed in v0.3.0
- Missing stdin forwarding - fixed in v0.3.0

### Earlier versions (v0.2.8 and below)
- Located in `src-tauri/target/release/bundle/msi/`
- Not organized - old build system

---

## Build Information

**Build System:** Tauri 2.2 + Vite 5 + SolidJS
**Rust Version:** 1.75+
**Node Version:** 18+

See `../../README.md` for version management guidelines and build instructions.
