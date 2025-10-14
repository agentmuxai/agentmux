# AgentMux Desktop Releases

All official desktop app releases are organized here by version.

## 📦 Latest Release: v0.3.0 (2025-10-14)

**Download:** [releases/v0.3.0/](v0.3.0/)

**Key Features:**
- ✅ **Fixed:** WebSocket stdin forwarding - UI messages now reach Claude CLI
- ✅ Embedded terminal with full Claude Code integration
- ✅ Agent spawning and management via UI
- ✅ Message bus interface
- ✅ Log export (text/JSON)

---

## 📥 Quick Download

Navigate to the version folder and choose your format:

- **Portable EXE** (recommended for testing)
  - No installation required
  - Run directly from any location
  - Size: ~19MB

- **MSI Installer**  
  - System-wide installation
  - Start menu integration
  - WebView2 auto-installed
  - Size: ~6MB

---

## 📋 All Releases

| Version | Date | Key Changes | Download |
|---------|------|-------------|----------|
| **v0.3.0** | 2025-10-14 | Fixed WebSocket stdin forwarding, organized releases | [Download](v0.3.0/) |
| v0.2.9 | 2025-10-13 | UI improvements (compact layout) | [Download](v0.2.9/) |
| v0.2.8 | 2025-10-13 | Bug fixes | [Download](v0.2.8/) |
| v0.2.7 | 2025-10-13 | Performance improvements | [Download](v0.2.7/) |
| v0.2.6 | 2025-10-13 | UI polish | [Download](v0.2.6/) |
| v0.2.5 | 2025-10-13 | Initial testing release | [Download](v0.2.5/) |
| v0.2.4 | 2025-10-13 | Alpha release | [Download](v0.2.4/) |
| v0.2.3 | 2025-10-13 | Early development | [Download](v0.2.3/) |

---

## 🔧 Installation Instructions

### Option 1: Portable Executable

```bash
# Download the portable exe
cd releases/v0.3.0
./agentmux-desktop-v0.3.0-portable.exe
```

No installation required - just run!

### Option 2: MSI Installer

1. Double-click `agentmux-desktop-v0.3.0-installer.msi`
2. Follow installation wizard
3. Find "AgentMux Desktop" in Start Menu

**Note:** WebView2 runtime will be automatically installed if not present.

---

## 🏗️ Build Instructions

Want to build from source? Follow these steps:

### 1. Version Bump (REQUIRED before each build)

```bash
cd apps/desktop

# Update package.json
npm version patch  # or minor, major

# Manually update these files to match package.json:
# - src-tauri/Cargo.toml (version = "0.3.0")
# - src-tauri/tauri.conf.json ("version": "0.3.0")
```

### 2. Build

```bash
# Build desktop app
npm run tauri:build

# Build output locations:
# - Portable EXE: apps/desktop/src-tauri/target/release/agentmux.exe
# - MSI Installer: apps/desktop/src-tauri/target/release/bundle/msi/
```

### 3. Organize Release

```bash
# Create version folder
mkdir -p releases/v0.X.Y

# Copy files
cp apps/desktop/src-tauri/target/release/agentmux.exe \
   releases/v0.X.Y/agentmux-desktop-v0.X.Y-portable.exe

cp "apps/desktop/src-tauri/target/release/bundle/msi/AgentMux Desktop_0.X.Y_x64_en-US.msi" \
   releases/v0.X.Y/agentmux-desktop-v0.X.Y-installer.msi
```

---

## ⚠️ Important Build Guidelines

**Every build MUST use a unique version number to avoid duplicates!**

1. **Update ALL 3 version files:**
   - `apps/desktop/package.json`
   - `apps/desktop/src-tauri/Cargo.toml`
   - `apps/desktop/src-tauri/tauri.conf.json` (CRITICAL - controls MSI filename)

2. **Why this matters:**
   - MSI filenames are based on `tauri.conf.json` version
   - Multiple builds with same version create duplicates
   - Confusion when distributing releases

3. **Version scheme:** Semantic versioning (X.Y.Z)
   - Patch: Bug fixes, minor changes (0.3.0 → 0.3.1)
   - Minor: New features, backwards compatible (0.3.1 → 0.4.0)
   - Major: Breaking changes (0.4.0 → 1.0.0)

---

## 📦 Release Folder Structure

```
releases/
├── README.md           # This file
├── v0.3.0/            # Latest release
│   ├── agentmux-desktop-v0.3.0-portable.exe
│   ├── agentmux-desktop-v0.3.0-installer.msi
│   └── README.md      # Version-specific notes
├── v0.2.9/            # Previous releases...
├── v0.2.8/
└── ...
```

---

## 🐛 Troubleshooting

### EXE won't run
- **Windows SmartScreen:** Click "More info" → "Run anyway"
- **Antivirus:** Add exception for AgentMux

### MSI installation fails
- **WebView2:** Ensure internet connection for auto-install
- **Permissions:** Run as administrator if needed

### App crashes on startup
- Check Windows Event Viewer for error details
- Ensure Claude CLI is installed and in PATH

---

## 📄 License

MIT

---

## 🔗 Links

- **Main README:** [../README.md](../README.md)
- **Build Guide:** [../apps/desktop/docs/BUILD.md](../apps/desktop/docs/BUILD.md)
- **Architecture:** [../apps/desktop/docs/ARCHITECTURE.md](../apps/desktop/docs/ARCHITECTURE.md)
- **Issues:** https://github.com/a5af/agentmux/issues
