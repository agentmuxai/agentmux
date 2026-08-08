# Portable Data Directory

**Date:** 2026-04-15  
**Author:** AgentA  
**Status:** Proposed (implemented — see note below)

> **2026-08-07 audit note:** Implemented (`agentmux-launcher/src/data_dir.rs`),
> a documented `task package` feature. Status field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

---

## Problem

In portable mode, AgentMux currently stores all user data in `~/.agentmux` — the same
location as an installed build. This has two problems:

1. **Not truly portable.** Extracting the ZIP to a USB drive or a different machine still
   writes to the host's home directory. Moving the folder loses nothing, but the data stays
   behind on the original machine.

2. **Tilde is ugly.** When a user opens the portable folder they see `runtime/` — a clean,
   purposeful name. The actual state lives invisibly somewhere else, which is confusing when
   diagnosing problems or doing a clean uninstall.

Installed builds (MSI, future package manager installs) should continue to use the platform
home directory. Only the portable layout needs to change.

---

## Proposed Layout

```
agentmux-0.33.185-x64-portable/
├── agentmux.exe          ← launcher (unchanged)
├── README.txt
├── data/                 ← NEW: all user state (was ~/.agentmux)
│   ├── db/               ← wave.db, block state
│   ├── config/           ← settings.json, keybindings.json
│   ├── logs/             ← host + sidecar log files
│   ├── cef/              ← CEF browser cache (was %LOCALAPPDATA%/ai.agentmux.cef.vX)
│   └── agents/           ← per-provider auth dirs, CLI installs
└── runtime/              ← binaries + CEF runtime DLLs (unchanged)
    ├── agentmux-0.33.185.exe
    ├── agentmux-srv-0.33.185-windows.x64.exe
    ├── libcef.dll
    └── ...
```

### Why `data/`

- Mirrors `runtime/` — two sibling directories, one for code, one for state
- Standard convention: used by Firefox (`profile/`), Chrome (`User Data/`),
  PortableApps (`Data/`), most Electron portable builds
- Short, obvious, no version number — survives if the user renames the parent folder
- Clearly distinct from `runtime/` (read-only shipped files vs. mutable user files)

---

## Detection

Portable mode is already implicitly detected by the presence of `runtime/` next to the
launcher. The same sentinel is used to activate the `data/` directory:

```
<exe_dir>/runtime/   exists  →  portable mode  →  data_dir = <exe_dir>/data/
<exe_dir>/runtime/   absent  →  installed mode →  data_dir = ~/.agentmux  (unchanged)
```

No new environment variable or flag is needed. The launcher (`agentmux.exe`) detects
`runtime/` today for DLL loading — the same check gates the data dir.

---

## Changes Required

### 1. `agentmux-cef/src/main.rs` — data_dir resolution

Replace the current `dirs::data_dir()` + version slug logic with:

```rust
let exe_dir = std::env::current_exe()?.parent()?.to_path_buf();
let runtime_dir = exe_dir.join("runtime");

let (data_dir, cef_cache_dir, is_portable) = if runtime_dir.exists() {
    // Portable mode: everything under <exe_dir>/data/
    let base = exe_dir.join("data");
    let cef = base.join("cef");
    (base, cef, true)
} else {
    // Installed mode: platform home dir (unchanged)
    let base = dirs::home_dir()
        .unwrap_or_default()
        .join(".agentmux");
    let cef_name = if is_dev {
        "ai.agentmux.cef.dev".to_string()
    } else {
        format!("ai.agentmux.cef.v{}", version_slug)
    };
    let cef = dirs::data_dir()
        .unwrap_or_default()
        .join(cef_name);
    (base, cef, false)
};
```

- `data_dir` → passed to sidecar as `AGENTMUX_DATA_HOME`
- `cef_cache_dir` → set as `CefSettings.root_cache_path`
- `is_portable` → used for log path (see below)

### 2. `agentmux-cef/src/main.rs` — log path

Logs currently hardcode `~/.agentmux/logs/`. Change to derive from `data_dir`:

```rust
let log_dir = data_dir.join("logs");
```

This ensures portable logs land in `data/logs/` not in the home dir.

### 3. `agentmux-cef/src/sidecar.rs` — env var pass-through

`AGENTMUX_DATA_HOME` is already passed to the sidecar. No change needed in the sidecar
launch code — the CEF host resolves the path and hands it down.

### 4. `scripts/package-portable.sh` — create `data/` scaffold

Create an empty `data/.gitkeep` (or just the `data/` directory) in the output folder so
users see it immediately on extraction and understand where their state lives:

```bash
mkdir -p "$OUT_DIR/data"
echo "# AgentMux user data — safe to back up, do not delete while app is running" \
    > "$OUT_DIR/data/README.txt"
```

### 5. `agentmux-srv/src/main.rs` — `.gitignore` creation

The backend creates `~/.agentmux/.gitignore` on first start. Ensure this uses the
resolved `data_dir` (already the case via `AGENTMUX_DATA_HOME` — no code change needed,
just verify).

---

## Migration (First-Run)

On first portable launch, if `data/` is empty and `~/.agentmux` exists, show a one-time
prompt in the agent pane / launch log:

> **Portable data directory created at `data/`.** Your existing sessions are in
> `~/.agentmux` — copy them here if you want to bring history along.

Do **not** auto-migrate. The user may be running multiple versions and the home dir copy
is their "installed" state — silently moving it could break the installed build.

---

## What Does NOT Change

| Thing | Stays the same |
|-------|----------------|
| Installed build data dir | `~/.agentmux` — untouched |
| Dev mode data dir | `~/.agentmux-dev` — untouched |
| CEF version isolation | In portable, single `data/cef/` (version is already in folder name) |
| `AGENTMUX_DATA_HOME` env override | Still highest priority, overrides everything |
| Multiple portable instances | Each extracted folder has its own `data/` — naturally isolated |

---

## Out of Scope

- **Symlink support** (`data/` → arbitrary path): out of scope for now; `AGENTMUX_DATA_HOME`
  covers the power-user case.
- **Auto-migration wizard**: out of scope; README.txt in `data/` is sufficient.
- **macOS / Linux portable**: same logic applies but not the focus of this spec — the
  `runtime/` sentinel works cross-platform.
