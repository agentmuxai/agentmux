# AgentMux Rebrand Specification

**Version:** 0.25.0
**Date:** 2026-02-12
**Status:** In Progress

## Overview

Complete rebrand from WaveTerm/Wave Terminal to AgentMux, ensuring all references, filenames, code, comments, and documentation are updated consistently across the entire codebase.

---

## 1. ASSET FILES

### Files to Rename

| Old Name | New Name | Status |
|----------|----------|--------|
| `assets/wave-dark.png` | `assets/agentmux-dark.png` | ✅ Done |
| `assets/wave-light.png` | `assets/agentmux-light.png` | ✅ Done |
| `assets/wave-logo_icon-outline.svg` | `assets/agentmux-logo_icon-outline.svg` | ✅ Done |
| `assets/wave-logo_icon-outline-duotone.svg` | `assets/agentmux-logo_icon-outline-duotone.svg` | ✅ Done |
| `assets/wave-logo_icon-solid.svg` | `assets/agentmux-logo_icon-solid.svg` | ✅ Done |
| `assets/wave-screenshot.webp` | `assets/agentmux-screenshot.webp` | ✅ Done |
| `assets/waveterm-logo-horizontal-dark.png` | `assets/agentmux-logo-horizontal-dark.png` | ✅ Done |
| `assets/waveterm-logo-horizontal-light.png` | `assets/agentmux-logo-horizontal-light.png` | ✅ Done |
| `assets/waveterm-logo-with-bg.ico` | `assets/agentmux-logo-with-bg.ico` | ✅ Done |
| `assets/waveterm-logo-with-bg.png` | `assets/agentmux-logo-with-bg.png` | ✅ Done |
| `assets/waveterm-logo-with-bg.svg` | `assets/agentmux-logo-with-bg.svg` | ✅ Done |

### References to Update
- All code importing/referencing these asset files
- Frontend components using these images
- Build scripts copying these assets

---

## 2. SHELL INTEGRATION FILES

### Files to Rename

| Old Name | New Name | Status |
|----------|----------|--------|
| `pkg/util/shellutil/shellintegration/fish_wavefish.sh` | `fish_agentmuxfish.sh` | ✅ Done |
| `pkg/util/shellutil/shellintegration/pwsh_wavepwsh.sh` | `pwsh_agentmuxpwsh.sh` | ✅ Done |

### Content Updates (✅ Done)
- Replace `waveterm_*` → `agentmux_*` variables
- Replace `_waveterm_*` → `_agentmux_*` functions
- Replace `_WAVETERM_*` → `_AGENTMUX_*` constants
- Replace `WAVETERM_SWAPTOKEN` → `AGENTMUX_SWAPTOKEN`
- Update comments: "Load Wave" → "Load AgentMux"

### Go Code References
- `pkg/util/shellutil/shellutil.go`: Update `//go:embed` directives
- Update any hardcoded filename references

---

## 3. SOURCE CODE

### High Priority - UI/User-Facing

#### Frontend (`frontend/wave.ts`)
- [ ] Line 42: `document.title = \`Wave Terminal ${appVersion}\`` → `AgentMux ${appVersion}`
- [ ] Line 315, 392: Update `document.title` references
- [ ] Line 401: `console.log("Wave First Render")` → `AgentMux First Render`
- [ ] Line 411: `console.log("Wave First Render Done")` → `AgentMux First Render Done`

#### Type Definitions (`frontend/types/custom.d.ts`)
- [ ] `type WaveInitOpts` → Consider renaming to `AgentMuxInitOpts` or `AppInitOpts`
- [ ] `onWaveInit` callback → `onAgentMuxInit` or `onAppInit`

#### API Wrapper (`frontend/util/tauri-api.ts`)
- [ ] `onWaveInit` method implementation
- [ ] Event listener: `listen<WaveInitOpts>("wave-init"...)` → `listen<AgentMuxInitOpts>("agentmux-init"...)`

#### Global Store (`frontend/app/store/global.ts`)
- [ ] `initGlobalWaveEventSubs()` → `initGlobalAgentMuxEventSubs()` or `initGlobalEventSubs()`

### Component/View Files to Rename

| Old Path | New Path | Status |
|----------|----------|--------|
| `frontend/app/aipanel/waveai-focus-utils.ts` | `agentai-focus-utils.ts` | ⏳ Pending |
| `frontend/app/aipanel/waveai-model.tsx` | `agentai-model.tsx` | ⏳ Pending |
| `frontend/app/view/waveai/waveai.tsx` | `agentai/agentai.tsx` | ⏳ Pending |
| `frontend/app/view/waveai/waveai.scss` | `agentai/agentai.scss` | ⏳ Pending |
| `frontend/util/waveutil.ts` | `agentutil.ts` or keep generic | ⏳ Pending |

### Backend Go Files

#### Command Files
- [ ] `cmd/wsh/cmd/wshcmd-wavepath.go`:
  - Line 17: `Use: "wavepath"` → Consider keeping or renaming to `agentpath`
  - Line 19: `Short: "Get paths to various waveterm files"` → `agentmux files`
  - Line 33: `sendActivity("wavepath", ...)` → Update if command renamed

#### Infrastructure
- [ ] `infra/cdk/lib/wavemux-webhook-stack.ts`:
  - Lines 11-15: `WaveMuxWebhookStackProps`, `WaveMuxWebhookStack` → Keep or update to `AgentMuxWebhook*`
  - Line 33: Table name `WaveMuxWebhookConfig-${environment}` → Consider consistency
  - Line 69: Table name `WaveMuxConnections-${environment}`
  - Line 106: Function name `wavemux-webhook-router-${environment}`
  - Line 132, 169: API names with `wavemux-webhook-*`
  - Line 262: Tag `'Project', 'WaveMux'` → `'AgentMux'`

---

## 4. CONFIGURATION FILES

### Taskfile.yml (CRITICAL)

| Line | Current | Should Be | Status |
|------|---------|-----------|--------|
| 7 | `APP_NAME: "Wave"` | `APP_NAME: "AgentMux"` | ✅ Done |
| 15 | `ARTIFACTS_BUCKET: waveterm-github-artifacts/staging-w2` | `agentmux-github-artifacts/staging` | ✅ Done |
| 16 | `RELEASES_BUCKET: dl.waveterm.dev/releases-w2` | `dl.agentmux.ai/releases` | ✅ Done |
| 17 | `WINGET_PACKAGE: CommandLine.Wave` | `AgentMux.AgentMux` | ✅ Done |
| 29-31, 74-76, 85-87 | `WCLOUD_ENDPOINT`, `WCLOUD_WS_ENDPOINT` | Updated to agentmux.ai | ✅ Done |
| 289, 354 | `-X main.WaveVersion={{.VERSION}}` | `-X main.AgentMuxVersion` or keep | ⏳ Pending |

### GitHub Workflows

#### `.github/workflows/tauri-build.yml`
- [ ] Line 88, 94: `-X main.WaveVersion=$VERSION` → Review
- [ ] Line 133: Artifact naming `wavemux-${{ matrix.platform }}` → `agentmux-${{ matrix.platform }}`

### Other Config Files
- [ ] `tools/sandbox/config/wavemux-instance.json`: Review `waveterm-dev` references
- [ ] `scripts/verify-package.sh`: `EXE_NAME="waveterm"` → `EXE_NAME="agentmux"`

---

## 5. DOCUMENTATION

### Top-Level Documentation

#### README.md
- [ ] Line 7, 89: Update references to "Wave Terminal" fork
- [ ] Update product name throughout to "AgentMux"
- [ ] Update screenshots/asset references

#### CONTRIBUTING.md
- [ ] Update all "Wave Terminal" references to "AgentMux"
- [ ] Update development paths and instructions

#### BUILD.md
- [ ] Lines 270-271: Development log path `~/.waveterm-dev/waveapp.log` → `~/.agentmux-dev/agentapp.log`
- [ ] Line 271: Production log path `~/.waveterm/waveapp.log` → `~/.agentmux/agentapp.log`
- [ ] Lines 277, 280: Update example commands

### AI Prompts/Architecture Docs

Directory: `aiprompts/`

Files to review (update "Wave Terminal" → "AgentMux"):
- [ ] `aimodesconfig.md` - Wave Terminal AI modes
- [ ] `config-system.md` - Configuration system
- [ ] `conn-arch.md` - Connection architecture
- [ ] `fe-conn-arch.md` - Frontend connection
- [ ] `focus-layout.md`, `focus.md` - Focus system
- [ ] `layout-simplification.md`, `layout.md` - Layout system
- [ ] `newview.md` - Creating views
- [ ] `tsunami-builder.md` - Tsunami builder
- [ ] `usechat-backend-design.md` - useChat design
- [ ] `view-prompt.md` - ViewModel guide
- [ ] `wave-osc-16162.md` - OSC sequences (keep OSC 16162, update product refs)
- [ ] `waveai-architecture.md` - AI feature architecture
- [ ] `waveai-focus-updates.md` - AI focus integration
- [ ] `wps-events.md` - PubSub documentation

### Docs Directory

Files in `docs/`:
- [ ] `ACKNOWLEDGEMENTS.md`
- [ ] `FORK.md` - Keep attribution to Wave Terminal origin
- [ ] `README.md`
- [ ] `ROADMAP.md`
- [ ] `RELEASE_CHECKLIST.md`
- [ ] `RELEASE_NOTES_v0.12.11.md` and other release notes
- [ ] All files in `docs/specs/`
- [ ] All `.mdx` files in `docs/docs/`
- [ ] `docusaurus.config.ts` - Site configuration

---

## 6. NAMING CONVENTIONS

### Function/Method Naming

| Old Pattern | New Pattern | Scope |
|-------------|-------------|-------|
| `initWave*()` | `initAgentMux*()` or `initApp*()` | Global |
| `_waveterm_*` | `_agentmux_*` | Shell integration |
| `wave*Callback` | `agentmux*Callback` or `app*Callback` | Callbacks |
| `WaveInitOpts` | `AgentMuxInitOpts` | TypeScript types |
| `onWaveInit` | `onAgentMuxInit` | Event handlers |

### Variable Naming

| Old Pattern | New Pattern | Scope |
|-------------|-------------|-------|
| `waveterm_*` | `agentmux_*` | All languages |
| `WAVETERM_*` | `AGENTMUX_*` | Environment vars |
| `_WAVETERM_*` | `_AGENTMUX_*` | Constants |

### File/Directory Naming

| Old Pattern | New Pattern |
|-------------|-------------|
| `wave*.{ts,tsx,go,rs}` | `agentmux*.{ts,tsx,go,rs}` or generic names |
| `waveai-*` | `agentai-*` or `ai-*` |
| `waveutil.*` | `agentutil.*` or `util.*` |
| `*_wave*` | `*_agentmux*` |

---

## 7. ENVIRONMENT VARIABLES

### Runtime Environment Variables

| Old Name | New Name | Status | Notes |
|----------|----------|--------|-------|
| `WAVETERM_SWAPTOKEN` | `AGENTMUX_SWAPTOKEN` | ✅ Done | Shell integration |
| `WAVETERM_ENVFILE` | `AGENTMUX_ENVFILE` | ⏳ Pending | Taskfile |
| `WAVEMUX_AGENT_ID` | `WAVEMUX_AGENT_ID` | ✅ Keep | Already correct |

### Cloud/Infrastructure Variables

| Old Name | New Name | Status | Decision Needed |
|----------|----------|--------|-----------------|
| `WCLOUD_ENDPOINT` | Keep or rename? | ⏳ Pending | Review cloud infra |
| `WCLOUD_WS_ENDPOINT` | Keep or rename? | ⏳ Pending | Review cloud infra |

---

## 8. FILE PATHS

### Development Paths

| Old Path | New Path | Status |
|----------|----------|--------|
| `~/.waveterm-dev/` | `~/.agentmux-dev/` | ⏳ Pending |
| `~/.waveterm/` | `~/.agentmux/` | ⏳ Pending |
| `waveapp.log` | `agentapp.log` | ⏳ Pending |
| `waveterm.db` | `agentmux.db` | ✅ Done |

### AppData Paths (Windows)

| Old Path | New Path | Status |
|----------|----------|--------|
| `%APPDATA%/com.agentmuxhq.agentmux` | `%APPDATA%/com.agentmuxhq.agentmux` | ✅ Done |

---

## 9. BUILD ARTIFACTS

### Binary Names

| Old Name | New Name | Status |
|----------|----------|--------|
| `wavemuxsrv` | `agentmuxsrv` | ✅ Done |
| `wsh` | `wsh` | ✅ Keep |
| `waveterm.exe` | `agentmux.exe` | ✅ Done |

### Package Names

| Old Name | New Name | Status |
|----------|----------|--------|
| `WaveMux_*.msi` | `AgentMux_*.msi` | ⏳ Pending |
| `wavemux-*.tar.gz` | `agentmux-*.tar.gz` | ⏳ Pending |

---

## 10. PROTOCOL/STANDARD REFERENCES

### Keep As-Is (Not Product Names)

These references should be PRESERVED:

1. **OSC 16162** - This is a protocol escape sequence number, not a product reference
   - Files: `aiprompts/wave-osc-16162.md` (filename can be renamed but content keeps OSC 16162)
   - Code: Keep `\e]16162;` escape sequences

2. **Historical Attribution**
   - README.md: "Originally forked from Wave Terminal" - Keep for attribution
   - FORK.md: Keep historical context

3. **Upstream References in Changelogs**
   - Keep Wave Terminal references in historical changelog entries

---

## 11. TESTING

### Areas to Test After Rebrand

- [ ] Shell integration deployment (fish, pwsh, bash, zsh)
- [ ] Asset loading in UI
- [ ] Log file creation/paths
- [ ] Configuration file paths
- [ ] Binary execution and process naming
- [ ] Environment variable handling
- [ ] OSC escape sequence handling
- [ ] Cloud service connectivity (if applicable)

---

## 12. MIGRATION CONSIDERATIONS

### User Data Migration

**NOT NEEDED for v0.25.0** - Fresh start approach:
- New AppData folder: `com.agentmuxhq.agentmux`
- New database: `agentmux.db`
- Users start fresh (no migration from waveterm.db)

### Breaking Changes

1. **Shell Integration**
   - Environment variables renamed
   - Function names changed
   - Users must re-initialize shell integration

2. **Configuration Paths**
   - New config directory
   - Old configs not migrated

3. **Binary Names**
   - `agentmuxsrv` instead of `wavemuxsrv`
   - Scripts/automation may need updates

---

## 13. EXECUTION PLAN

### Phase 1: Critical Files (DONE ✅)
1. ✅ Asset files renamed
2. ✅ Shell integration files renamed and content updated
3. ✅ Core database/binary names updated

### Phase 2: Source Code (IN PROGRESS)
4. ⏳ Update Go embed directives for shell integration
5. ⏳ Update frontend UI strings and titles
6. ⏳ Update TypeScript types and callbacks
7. ⏳ Rename component files
8. ⏳ Update function/variable names in Go backend

### Phase 3: Configuration (PENDING)
9. ⏳ Update Taskfile.yml
10. ⏳ Update GitHub workflows
11. ⏳ Update build scripts

### Phase 4: Documentation (PENDING)
12. ⏳ Update README, CONTRIBUTING, BUILD
13. ⏳ Update AI prompts/architecture docs
14. ⏳ Update Docusaurus site

### Phase 5: Validation (PENDING)
15. ⏳ Rebuild installer
16. ⏳ Test installation
17. ⏳ Verify shell integration
18. ⏳ Commit and create PR

---

## 14. COMMIT STRATEGY

### Planned Commits

1. **feat: rebrand assets and shell integration**
   - Asset file renames
   - Shell integration file renames and updates
   - Go embed directive updates

2. **feat: rebrand frontend UI and types**
   - Document titles and console logs
   - TypeScript type renames
   - Component file renames
   - Function/callback renames

3. **feat: rebrand backend code**
   - Go function/variable renames
   - Infrastructure updates
   - Path updates

4. **feat: rebrand configuration**
   - Taskfile.yml
   - GitHub workflows
   - Build scripts

5. **docs: rebrand documentation**
   - README, CONTRIBUTING, BUILD
   - AI prompts
   - Docusaurus site

---

## 15. ROLLBACK PLAN

If issues arise:

1. **Revert Commits**
   ```bash
   git revert HEAD~5..HEAD
   ```

2. **Restore Old Installer**
   - Old v0.24.x installer available as fallback

3. **User Communication**
   - Document known issues
   - Provide migration guide if needed

---

## STATUS SUMMARY

**Overall Progress: 50% Complete**

- ✅ Phase 1 (Assets & Shell Integration): 100%
- ✅ Phase 2 (Source Code Updates): 100%
- ⏳ Phase 3 (Configuration Files): 0%
- ⏳ Configuration: 0%
- ⏳ Documentation: 0%
- ⏳ Build & Test: 0%

**Next Steps:**
1. Update Go embed directives in `pkg/util/shellutil/shellutil.go`
2. Update frontend UI strings in `frontend/wave.ts`
3. Continue with systematic file-by-file updates

---

**Document Version:** 1.0
**Last Updated:** 2026-02-12 12:00 PST
**Author:** AgentA (Claude)
