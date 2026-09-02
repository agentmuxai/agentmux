# Spec: Agent Tool Store — managed CLI tool availability for agent panes
**Date:** 2026-04-15
**Status:** Draft
**Scope:** `agentmux-srv/src/backend/tool_store.rs` (new),
`agentmux-srv/src/backend/blockcontroller/shell.rs`,
`agentmux-srv/src/server/` (new RPC handler),
`frontend/app/view/agent/hooks/useAgentCommands.ts`,
`frontend/app/view/agent/commands/` (new `/tools` slash command),
`agentmux-srv/src/config/tool-catalog.json` (new)

---

## 1. Problem

When AgentMux spawns an agent subprocess, the process inherits the host's
`PATH`. On a fresh machine — or a dev VM, or a user who installs software
in non-standard locations — tools the agent depends on may simply be absent:

```
$ jq --version
command not found: jq

$ fd --help
command not found: fd
```

Claude Code itself works fine, but any tool call that shells out to `jq`, `fd`,
`rg`, `bat`, `delta`, or `gh` silently fails with a cryptic error that the
agent (and user) must debug. On Windows this is worse — none of these tools
ship with the OS, and most users have never heard of Scoop.

**Goals:**
1. The agent should always have a baseline set of useful CLI tools available,
   regardless of what the host has installed.
2. Tools should be installed once, cheaply, without admin rights.
3. Users should see which tools are available (and missing) at a glance.
4. The system must not break if a tool download fails — degraded gracefully.

---

## 2. Tool catalog

Tools are divided into three tiers:

### Tier 1 — Essential (always checked, auto-install prompt on first agent launch)

| Tool | Purpose | Why it matters for agents |
|------|---------|--------------------------|
| `jq` | JSON processor | Parse API responses, filter structured output — used in ~40% of real Claude sessions |
| `rg` (ripgrep) | Fast recursive search | Claude Code uses this for Grep; absence causes fallback to slow `grep` |
| `gh` | GitHub CLI | PRs, issues, releases — most forge workflows need it |

### Tier 2 — Recommended (shown in `/tools` UI, one-click install)

| Tool | Purpose |
|------|---------|
| `fd` | Fast `find` replacement — cleaner API, respects `.gitignore` |
| `bat` | `cat` with syntax highlighting and line numbers |
| `delta` | Beautiful `git diff` pager |
| `yq` | YAML/TOML processor (mirrors `jq`) |
| `fzf` | Fuzzy finder — autocomplete scripts, interactive filters |

### Tier 3 — Optional (documented only, user installs via native package manager)

| Tool | Install hint |
|------|-------------|
| `ffmpeg` | `winget install Gyan.FFmpeg` / `brew install ffmpeg` |
| `imagemagick` | `winget install ImageMagick.ImageMagick` |
| `pandoc` | `winget install JohnMacFarlane.Pandoc` |

Only Tiers 1 and 2 are managed by AgentMux. Tier 3 is documented in the
agent's CLAUDE.md hint section.

---

## 3. Tool store layout — two tiers of resolution

There are **two** places tools can live. The PATH injection in `shell.rs`
checks both and appends them (system PATH still wins for tools the user has
explicitly installed):

```
PATH order for agent subprocesses:
  system PATH  (user's own tools — always wins)
  ↓
  ~/.agentmux/tools/bin/    ← user-managed store: downloaded on demand
  ↓
  {agentmux_exe_dir}/tools/bin/   ← bundled store: ships with the app
```

### 3.1 Bundled store — ships inside every release

Tier-1 tools (`jq`, `rg`) are **pre-compiled and bundled** inside the release
artifact so agents work offline, on first run, with zero user action.

```
# Windows portable ZIP layout
agentmux-VERSION-x64-portable/
├── agentmux.exe              (launcher)
└── runtime/
    ├── agentmux-VERSION.exe  (CEF host)
    ├── agentmux-srv-*.exe
    ├── libcef.dll  ...
    ├── frontend/
    └── tools/
        └── bin/
            ├── jq.exe
            └── rg.exe

# macOS .app layout  (once macOS packaging is implemented)
AgentMux.app/
└── Contents/
    └── Resources/
        └── tools/
            └── bin/
                ├── jq
                └── rg

# Linux AppImage / .deb  (once Linux packaging is implemented)
/usr/lib/agentmux/tools/bin/   ← .deb
<AppImage>/tools/bin/          ← AppImage (squashfs root)
```

`agentmux-srv` resolves its own executable path at startup
(`std::env::current_exe()`) and walks up to find `tools/bin/` relative to it:

```rust
fn bundled_tools_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // srv binary is at runtime/agentmux-srv-*.exe (Windows portable)
    // or  /usr/bin/agentmux-srv (installed)
    // Walk: exe → parent (runtime/) → join("tools/bin")
    let candidate = exe.parent()?.join("tools").join("bin");
    if candidate.exists() { Some(candidate) } else { None }
}
```

This is the same pattern the launcher already uses to find `runtime/`.

**Packaging steps** (per platform):
- `scripts/package-portable.sh` — add a `copy_bundled_tools_windows` step
  that downloads (at build time) `jq.exe` and `rg.exe` into `$PORTABLE/runtime/tools/bin/`
- macOS packaging (future) — same, `tools/bin/jq` + `tools/bin/rg` (arm64 or x64 based on build target)
- Linux packaging (future) — musl-static builds of jq + rg

Build-time downloads are guarded by a lockfile `scripts/tool-versions.json`
that pins exact URLs + SHA-256s, so CI is reproducible and auditable.

### 3.2 User-managed store — downloaded on demand

`~/.agentmux/tools/bin/` is the per-user layer. It holds Tier-2 tools
(`fd`, `bat`, `delta`, `yq`, `fzf`) and updated versions of Tier-1 tools
that the user installs via `/tools install`.

```
~/.agentmux/tools/
├── bin/
│   ├── fd[.exe]
│   ├── bat[.exe]
│   └── ...
├── versions.json       ← installed tool IDs + versions + sha256
└── downloads/          ← temp staging, cleaned after extract
```

### 3.3 PATH injection point

In `shell.rs` at the `AGENTMUX=1` line (~506), append both store dirs:

```rust
let sep = if cfg!(windows) { ";" } else { ":" };
let current_path = std::env::var("PATH").unwrap_or_default();
let mut extra: Vec<String> = Vec::new();

// User-managed store (downloaded tools)
let user_store = dirs::home_dir()
    .unwrap_or_default()
    .join(".agentmux").join("tools").join("bin");
if user_store.exists() { extra.push(user_store.to_string_lossy().into()); }

// Bundled store (ships with app, works offline)
if let Some(bundled) = bundled_tools_dir() { extra.push(bundled.to_string_lossy().into()); }

if !extra.is_empty() {
    // Append — system PATH takes precedence so user's own tools always win
    c.env("PATH", format!("{current_path}{sep}{}", extra.join(sep)));
}
```

No-op if neither directory exists — fully graceful.

---

## 4. Tool download config (`tool-catalog.json`)

Stored at `agentmux-srv/src/config/tool-catalog.json`, embedded via
`include_str!` into the binary so there are no file-system lookups at runtime.

```json
{
  "version": 1,
  "tools": [
    {
      "id": "jq",
      "display": "jq",
      "description": "Lightweight JSON processor",
      "tier": 1,
      "check": ["jq", "--version"],
      "platforms": {
        "windows-x64": {
          "url": "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-windows-amd64.exe",
          "sha256": "...",
          "extract": "none",
          "bin": "jq.exe"
        },
        "macos-arm64": {
          "url": "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-macos-arm64",
          "sha256": "...",
          "extract": "none",
          "bin": "jq",
          "chmod_x": true
        },
        "macos-x64": {
          "url": "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-macos-amd64",
          "sha256": "...",
          "extract": "none",
          "bin": "jq",
          "chmod_x": true
        },
        "linux-x64": {
          "url": "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64",
          "sha256": "...",
          "extract": "none",
          "bin": "jq",
          "chmod_x": true
        }
      }
    },
    {
      "id": "rg",
      "display": "ripgrep",
      "description": "Fast recursive grep — used by Claude Code's Grep tool",
      "tier": 1,
      "check": ["rg", "--version"],
      "platforms": {
        "windows-x64": {
          "url": "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep-14.1.1-x86_64-pc-windows-msvc.zip",
          "sha256": "...",
          "extract": "zip",
          "bin_in_archive": "ripgrep-14.1.1-x86_64-pc-windows-msvc/rg.exe",
          "bin": "rg.exe"
        }
        // ... macos, linux
      }
    },
    {
      "id": "gh",
      "display": "GitHub CLI",
      "description": "PR, issue, and release management",
      "tier": 1,
      "check": ["gh", "--version"],
      "platforms": {
        "windows-x64": {
          "url": "https://github.com/cli/cli/releases/download/v2.67.0/gh_2.67.0_windows_amd64.zip",
          "sha256": "...",
          "extract": "zip",
          "bin_in_archive": "gh_2.67.0_windows_amd64/bin/gh.exe",
          "bin": "gh.exe"
        }
        // ... macos, linux
      }
    }
    // fd, bat, delta, yq, fzf follow same pattern
  ]
}
```

**Extract modes:**
- `none` — file is the binary; copy directly to `bin/`
- `zip` — extract archive, copy specific `bin_in_archive` path to `bin/`
- `tar.gz` — same, different archive format

---

## 5. Backend module: `tool_store.rs`

New module at `agentmux-srv/src/backend/tool_store.rs`.

```rust
pub struct ToolStore {
    home: PathBuf,            // ~/.agentmux/tools
    catalog: ToolCatalog,     // parsed from embedded JSON
}

impl ToolStore {
    pub fn new() -> Self { ... }

    /// Check which tools are missing from PATH or the managed store.
    /// Returns a list of missing tool IDs.
    pub fn check_missing(&self) -> Vec<String> { ... }

    /// Install a single tool by ID. Downloads, verifies sha256, extracts.
    /// Progress reported via a callback (sent over RPC to frontend).
    pub async fn install(&self, id: &str, progress: impl Fn(ToolProgress)) -> Result<()> { ... }

    /// Read versions.json — returns installed tool versions.
    pub fn installed_versions(&self) -> HashMap<String, String> { ... }

    /// Get the full tool catalog (for frontend display).
    pub fn catalog(&self) -> &ToolCatalog { ... }
}

pub struct ToolProgress {
    pub tool_id: String,
    pub phase: ToolInstallPhase,     // Downloading | Verifying | Extracting | Done | Failed
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub error: Option<String>,
}
```

### 5.1 Check logic

`check_missing` resolves each tool's `check` command:
1. Try `which jq` (or `where jq` on Windows) — found in system PATH → not missing
2. Try `~/.agentmux/tools/bin/jq` — exists → not missing
3. Neither → missing

Tools already on the system PATH are never re-downloaded — we only fill gaps.

### 5.2 Download + verify

```rust
async fn install(&self, id: &str, progress: impl Fn(ToolProgress)) -> Result<()> {
    let spec = self.catalog.tool(id)?;
    let platform = current_platform()?; // "windows-x64" | "macos-arm64" | ...
    let download = spec.platform(platform)?;

    // 1. Download to downloads/ temp file
    let tmp = self.home.join("downloads").join(format!("{id}-tmp"));
    http_download(&download.url, &tmp, |done, total| {
        progress(ToolProgress { tool_id: id.to_string(),
            phase: ToolInstallPhase::Downloading, bytes_done: done, bytes_total: total, error: None });
    }).await?;

    // 2. SHA-256 verify
    progress(ToolProgress { ..., phase: ToolInstallPhase::Verifying });
    verify_sha256(&tmp, &download.sha256)?;

    // 3. Extract
    progress(ToolProgress { ..., phase: ToolInstallPhase::Extracting });
    let bin_src = match &download.extract {
        Extract::None => tmp.clone(),
        Extract::Zip => extract_zip(&tmp, &download.bin_in_archive, &self.home.join("downloads"))?,
        Extract::TarGz => extract_targz(...),
    };

    // 4. Copy to bin/
    let dest = self.home.join("bin").join(&download.bin);
    fs::copy(bin_src, &dest)?;
    #[cfg(unix)] { set_executable(&dest)?; }

    // 5. Update versions.json
    self.record_installed(id, &spec.version)?;

    // Cleanup
    fs::remove_dir_all(self.home.join("downloads"))?;

    progress(ToolProgress { ..., phase: ToolInstallPhase::Done });
    Ok(())
}
```

---

## 6. RPC interface

New commands added to the RPC layer:

```
CommandGetToolStatus   → ToolStatusResponse   (list of tools + installed/missing/version)
CommandInstallTool     → streaming progress   (uses server-sent events or WPS file subject)
```

### `CommandGetToolStatus`

```json
// Response
{
  "tools": [
    {
      "id": "jq",
      "display": "jq",
      "description": "...",
      "tier": 1,
      "status": "missing",        // "installed_managed" | "installed_system" | "missing"
      "version": null,            // null if missing
      "system_path": null
    },
    {
      "id": "rg",
      "status": "installed_system",
      "version": "14.1.0",
      "system_path": "/usr/bin/rg"
    }
  ]
}
```

### `CommandInstallTool`

Accepts `{ "tool_ids": ["jq", "rg"] }`. Streams `ToolProgress` events back
via a WPS file subject (same mechanism as agent output streaming). Frontend
subscribes and renders progress bars.

---

## 7. Frontend: `/tools` slash command

New slash command registered in `commands/global/tools.ts`:

```
/tools                       → open the tool status panel
/tools install               → install all missing Tier 1 tools
/tools install jq rg         → install specific tools
/tools status                → print tool status to the chat
```

### `/tools status` output (rendered in the agent pane)

```
Tool availability:

✓ rg 14.1.0  (system)
✓ gh 2.67.0  (system)
✗ jq         not found — run /tools install jq
✗ fd         not found — run /tools install fd
```

### `/tools install` flow

1. Call `CommandInstallTool` with the list of missing tier-1 tools.
2. Render a progress node in the agent document:
   ```
   Installing tools...
   ⬇ jq     [████████████████░░░░] 1.2MB / 1.5MB
   ⬇ fd     waiting...
   ```
3. On completion, recheck and print the new status.

---

## 8. Agent launch integration

### 8.1 Launch log warning

When `useAgentControllerStatus` starts the launch flow, before spawning the
CLI, call `CommandGetToolStatus` and log a warning for missing Tier-1 tools:

```
[tools] ⚠ jq not found — run /tools install to fix
```

This appears in the launch log section (the terminal-style lines that appear
before the first agent message). It is a warning, not a blocker.

### 8.2 First-launch modal (optional, v2)

On the very first agent launch on a machine (detected by a flag in
`~/.agentmux/tool_store_initialized`), show a one-time modal:

> **Recommended tools missing**
> jq, fd, fzf — these make your agents significantly more capable.
> [Install now — ~5MB] [Skip]

"Install now" triggers `CommandInstallTool` for all Tier-1 tools in the
background, with a small progress indicator in the status bar.

---

## 9. CLAUDE.md injection

When AgentMux generates a CLAUDE.md for a forge agent
(`launchForgeAgent` in `agent-model.ts`), append a "Tool availability"
section:

```markdown
## Available tools

The following CLI tools are pre-installed by AgentMux and always on PATH:
- **jq** — JSON processor: `jq '.key' file.json`
- **rg** — Fast search: `rg pattern src/`
- **gh** — GitHub CLI: `gh pr list`, `gh issue create`
- **fd** — Fast file find: `fd 'pattern' src/`
- **bat** — cat with highlighting: `bat file.ts`
- **yq** — YAML processor: `yq '.key' config.yaml`

(Generated by AgentMux tool store — install more with `/tools install`)
```

This is only injected when the tool is actually installed (i.e. exists in
`~/.agentmux/tools/bin/` or system PATH). No fake promises.

---

## 10. Security considerations

- **SHA-256 checksums** are hardcoded in `tool-catalog.json` (not fetched from the
  same server as the binary). This prevents a compromised CDN from silently
  replacing binaries.
- Downloads go over HTTPS only (no HTTP fallback).
- Extracted files are placed only in `~/.agentmux/tools/bin/` — never in
  system directories. No admin rights required.
- `tool-catalog.json` is embedded in the binary at build time. It can only
  be updated by shipping a new AgentMux version — no remote update surface.

---

## 11. Platform matrix

### 11.1 Platform detection

`current_platform()` in `tool_store.rs` returns one of six keys:

```rust
fn current_platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x64"),
        ("macos",   "aarch64") => Ok("macos-arm64"),
        ("macos",   "x86_64") => Ok("macos-x64"),
        ("linux",   "x86_64") => Ok("linux-x64"),
        ("linux",   "aarch64") => Ok("linux-arm64"),
        (os, arch) => Err(format!("unsupported platform: {os}/{arch}")),
    }
}
```

The same six keys are used as property names in `tool-catalog.json`. A tool
entry that omits a platform key simply isn't available on that platform —
`/tools status` shows it as "not available on this platform" rather than
"missing".

### 11.2 Windows

| Topic | Detail |
|-------|--------|
| Binary format | `.exe` — no chmod needed |
| VC runtime | All recommended tools (jq, rg, fd, bat) ship as MSVC or MinGW static builds — no VC redist dependency |
| `gh` | Requires Git for Windows for its credential helper — already a Claude Code prerequisite; link to that check, don't duplicate |
| Path separator | `;` — handled in `PATH` append |
| Bundled in portable | `runtime/tools/bin/jq.exe`, `runtime/tools/bin/rg.exe` |
| Packaged installer | NSIS installer copies `tools/bin/` to `%ProgramFiles%\AgentMux\tools\bin\` |

### 11.3 macOS

| Topic | Detail |
|-------|--------|
| Architectures | `arm64` (Apple Silicon) + `x64` (Intel) — separate catalog entries, detected at runtime via `std::env::consts::ARCH` |
| chmod | All downloaded binaries need `chmod +x` after copy to `bin/` |
| Gatekeeper quarantine | GitHub-release binaries get `com.apple.quarantine` xattr when downloaded by a process that isn't already exempt. Remove with: `xattr -d com.apple.quarantine <file>` immediately after copy, before first execution. Failure is non-fatal — Gatekeeper may prompt the user once instead. |
| Code signing | Tools from GitHub are signed by their authors (jq, rg, gh, fd are all notarized for macOS). No re-signing needed. |
| Bundled in .app | `AgentMux.app/Contents/Resources/tools/bin/jq` + `rg` (arm64 or x64 matching the app build) |
| macOS packaging | Not yet implemented (`task package:macos` is a stub). When it lands, the packaging script fetches the correct arch binaries at build time, same as Windows portable. |

### 11.4 Linux

| Topic | Detail |
|-------|--------|
| Architectures | `x64` (amd64) + `arm64` (aarch64 — AWS Graviton, Raspberry Pi 4/5, Apple Silicon VMs) |
| libc | Use **musl-static** builds everywhere — no glibc version matching headache. All tier-1 and tier-2 tools publish musl-static binaries on their GitHub releases. |
| chmod | Same as macOS — `chmod +x` after copy |
| Gatekeeper / quarantine | None on Linux |
| AppImage | Not yet implemented (`task package:portable:linux` is a stub). When it lands, `tools/bin/` is part of the squashfs root so tools are available offline. `agentmux-srv` finds them via `bundled_tools_dir()` same as Windows. |
| .deb | Installs to `/usr/lib/agentmux/tools/bin/` — `agentmux-srv` walks up from its own exe path. |
| Snap / Flatpak | Out of scope for v1 — sandboxed runtimes restrict `execve`, making bundled CLI tools complicated. Document as "install system tools via apt" for Snap users. |

### 11.5 Deployment context summary

| Deployment | Bundled tools available offline? | User-managed store? | Notes |
|------------|----------------------------------|---------------------|-------|
| Windows portable ZIP | ✓ `runtime/tools/bin/` | ✓ `~/.agentmux/tools/bin/` | Both tiers work day-one |
| Windows NSIS installer | ✓ `%ProgramFiles%\AgentMux\tools\bin\` | ✓ | Same |
| macOS .app (future) | ✓ `Resources/tools/bin/` | ✓ | Requires `xattr` quarantine removal at build-copy time |
| Linux AppImage (future) | ✓ squashfs `tools/bin/` | ✓ | musl-static only |
| Linux .deb (future) | ✓ `/usr/lib/agentmux/tools/bin/` | ✓ | |
| `task dev` (dev mode) | ✗ (no bundled dir in dev) | ✓ | Dev users typically have tools installed; no bundle needed |

**Dev mode note:** `bundled_tools_dir()` returns `None` in `task dev` because
the srv binary is at `target/debug/agentmux-srv` with no `tools/` sibling.
That's intentional — dev users manage their own tools. The user-managed store
(`~/.agentmux/tools/bin/`) still works in dev mode.

---

## 12. Implementation steps

### Phase 1 — Bundled baseline (works offline, zero user action)

| Step | File | Notes |
|------|------|-------|
| 1a | `scripts/tool-versions.json` | Pin exact URLs + SHA-256s for jq + rg, all 6 platform variants |
| 1b | `scripts/package-portable.sh` | At package time: download + verify jq.exe + rg.exe into `$PORTABLE/runtime/tools/bin/` |
| 1c | `backend/tool_store.rs` | `bundled_tools_dir()` — walk up from srv exe to find `tools/bin/` sibling |
| 1d | `backend/blockcontroller/shell.rs` | Append both store dirs to subprocess PATH |

Phase 1 alone gives every Windows portable user jq + rg with no UI changes.
macOS and Linux get the same when their packaging tasks are implemented.

### Phase 2 — User-managed store + UI

| Step | File | Notes |
|------|------|-------|
| 2a | `tool-catalog.json` | Full catalog: all 8 tools, all 6 platforms, correct sha256s |
| 2b | `backend/tool_store.rs` | `check_missing`, `install`, `installed_versions`, platform detection |
| 2c | `server/tool_handlers.rs` | `GetToolStatus` + streaming `InstallTool` RPC |
| 2d | `commands/global/tools.ts` | `/tools` slash command |
| 2e | `hooks/useAgentControllerStatus.ts` | Log Tier-1 warnings at launch |
| 2f | `agent-model.ts` | Inject available tool list into generated CLAUDE.md |

### Phase 3 — Polish (v2)

| Step | Notes |
|------|-------|
| First-launch modal | After phases 1+2 are proven stable |
| macOS .app bundling | Wire into macOS packaging task when it's implemented |
| Linux AppImage/deb bundling | Wire into Linux packaging tasks when implemented |

Steps 1–4 (backend + PATH injection) can ship as a silent improvement with
no frontend changes. Steps 5–7 add the user-visible surface. Step 8 is polish.

---

## 13. What this does NOT change

- The system PATH is never modified — only the subprocess environment.
- Existing tool installs (user's own jq, rg, etc.) take precedence when they
  appear in the system PATH before `~/.agentmux/tools/bin/`. Wait — actually
  the spec inverts this: managed store is prepended, so it wins. If a user
  has jq 1.6 installed system-wide and the store has 1.7.1, the store wins.
  **Decision:** user's system PATH should take precedence over the managed
  store for tools they've explicitly installed. Append managed store instead
  of prepend, OR skip download for tools that already resolve in `which`.
  The `check_missing` function already does this (step 1: check system PATH
  first). So the managed store only fills gaps, never shadows.

  **Revised PATH strategy:** append (not prepend) `~/.agentmux/tools/bin/`
  so system-installed tools always win. This also means the user can override
  a managed tool by installing a newer version on their system PATH.

- No changes to the Claude Code, Codex, or Gemini CLI install flows — those
  are in `cli_handlers.rs` and stay separate.
- No new network calls unless the user explicitly runs `/tools install`.

---

## 14. Open questions

1. **Auto-update?** Should the tool store check for newer versions of managed
   tools periodically? — Lean no for v1. Tools like jq 1.7.1 are stable for
   years. Shipping a new AgentMux version is the update mechanism. Re-evaluate
   if users report version issues.

2. **Offline mode?** Resolved by the two-tier design: jq + rg are bundled in
   the release artifact (Phase 1), so they work with zero network access.
   Tier-2 tools require network only when the user explicitly runs
   `/tools install` — a deliberate user action, so a network failure at
   that point produces a clear error, not a silent degradation.

3. **Version pinning in tool-catalog.json** — should versions be configurable
   by the user? — No; too much surface. The catalog is baked into the binary.

4. **Windows: where does Git for Windows fit?** `gh` requires it. Should
   AgentMux check for Git and warn if absent? — Already handled by the Claude
   Code prerequisite check; add a cross-reference, not a duplicate check.
