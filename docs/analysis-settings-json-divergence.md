# Analysis: Why Dev and Production settings.json Look Wildly Different

**Date:** 2026-03-14
**Reported by:** asaf
**Symptom:** Opening settings.json from the widget in dev mode shows ~14 lines of uncommented JSON. Opening it in production shows ~84 lines of commented JSONC template. They should come from the same template.

---

## Root Cause

Three independent design decisions compound to create this divergence:

### 1. Tauri `app_config_dir()` is identifier-specific

Tauri resolves `app_config_dir()` using the app's `identifier` field from `tauri.conf.json`:

| Build | Identifier | Config Dir |
|-------|-----------|------------|
| Dev | `ai.agentmux.app.dev` | `AppData\Roaming\ai.agentmux.app.dev\` |
| v0.31.130 | `ai.agentmux.app.v0-31-130` | `AppData\Roaming\ai.agentmux.app.v0-31-130\` |
| v0.31.131 | `ai.agentmux.app.v0-31-131` | `AppData\Roaming\ai.agentmux.app.v0-31-131\` |

Every version bump creates a **brand new, empty directory**. Dev always reuses the same one.

### 2. `ensure_settings_file` skips existing files

```rust
// src-tauri/src/commands/platform.rs:155-158
let settings_path = config_dir.join("settings.json");
if !settings_path.exists() {
    std::fs::write(&settings_path, DEFAULT_SETTINGS_TEMPLATE)?;
}
```

This was added in commit `384af64` (2026-03-06) by `agentx-workflow[bot]`. At that time, the template was just `"{\n}\n"` — a minimal bootstrap. The guard made sense: don't overwrite user edits.

### 3. The template evolved but old files were never updated

| Commit | Date | Template Change |
|--------|------|----------------|
| `384af64` | Mar 6 | Initial: `"{\n}\n"` (2 lines) |
| `6372ec5` / `d337b4f` | Mar 7 | JSONC commented template (~81 lines) |
| `e262235` | Mar 7 | Added `widget:icononly` |
| `0e71c0e` | Mar 10 | Added `telemetry:numpoints` |

The dev `settings.json` was created **before** the template overhaul (when it was just `{}`). User interactions (widget drag/drop, hide/show) added real keys to it. The `!exists()` guard prevented the new template from ever being written.

Production builds, by contrast, get a fresh directory each version bump → `settings.json` doesn't exist → latest template is written.

---

## Current State on Disk

### Dev (`ai.agentmux.app.dev`) — 14 lines, created pre-template
```json
{
  "widget:hidden@defwidget@agent": true,
  "widget:hidden@defwidget@forge": true,
  "widget:icononly": false,
  "widget:order": ["agent", "forge", "swarm", "settings", "sysinfo", "help", "terminal", "devtools"]
}
```
This is **live user config** — actual settings written by the app via `SetMetaCommand`. No comments, no template structure.

### Production v0.31.130 — 84 lines, fresh template
```jsonc
// AgentMux Settings
// Save this file to apply changes immediately.
// Uncomment a line to override its default value.
//
// Docs: https://docs.agentmux.ai/settings
{
    // -- Terminal --
    // "term:fontsize":            12,
    // ... (all settings commented out)
}
```
This is the **reference template** — all defaults commented out, nothing active.

### Anomalous production versions (v0.31.104, v0.31.105) — 4-5 lines
These were created by app interactions (widget visibility toggling) **before** the user opened the settings widget. The `ensure_settings_file` command only runs when the widget is clicked, so these files were written by the backend's `SetMetaCommand` RPC directly, bypassing the template entirely.

### Full inventory (19 production directories)

| Version | Lines | Notes |
|---------|-------|-------|
| v0.31.62 | 2 | Pre-template era (`{}`) |
| v0.31.68–77 | 81 | First JSONC template |
| v0.31.79 | 82 | Template grew |
| v0.31.81–88 | 83 | +1 setting added |
| v0.31.100–103 | 83-84 | Current template |
| v0.31.104 | 5 | App-written, user never opened widget |
| v0.31.105 | 4 | App-written, user never opened widget |
| v0.31.130 | 84 | Current template |

---

## The Dual-Purpose File Problem

`settings.json` serves two conflicting roles:

1. **Reference template** — a commented-out catalog of all available settings, intended for human reading
2. **Active configuration** — the file the backend watches and applies settings from

These are at odds:
- Always overwriting ensures users see the latest template → but destroys their customizations
- Never overwriting preserves customizations → but the template goes stale
- The backend (`config_watcher_fs.rs`) reads AND writes this same file via `merge_settings_to_disk()`

### How the backend writes settings

When the frontend calls `RpcApi.SetMetaCommand()` (e.g., toggling widget visibility), the backend's `merge_settings_to_disk()` in `config_watcher_fs.rs` reads the existing file, merges the new keys, and writes it back. This **strips all comments** because `serde_json` doesn't preserve JSONC comments. So even if a user starts with the 84-line template and the app writes one setting, the file becomes raw JSON with no comments.

---

## Proposed Solutions

### Option A: Separate template from config (recommended)
- `settings.json` — active config only, written/read by the backend
- `settings.template.jsonc` — read-only reference, always overwritten from `DEFAULT_SETTINGS_TEMPLATE`
- The widget opens BOTH files side-by-side (or opens the template with a header comment pointing to `settings.json`)
- Pro: Clean separation, no data loss risk
- Con: Two files to manage

### Option B: Merge strategy on ensure
- On `ensure_settings_file`: read existing settings, deep-merge user values into the latest template, write back
- Preserves both the template structure and user customizations
- Pro: Single file
- Con: Complex, fragile with JSONC comments, `serde_json` round-trip strips comments anyway

### Option C: Always overwrite template, backend writes to separate file
- `ensure_settings_file` always writes the latest template (current change)
- Backend writes user settings to `settings.active.json` or similar
- Config watcher merges both: template defaults + active overrides
- Pro: Template always fresh
- Con: Requires backend refactor

### Option D: Write template only if file matches a known old template
- Hash the existing file, compare against known old templates
- Only overwrite if it's an unmodified old template
- If user has customized it, leave it alone
- Pro: Safe, no data loss
- Con: Doesn't solve the stale template problem for customized files

---

## Immediate Impact

The current always-overwrite change (removing the `!exists()` guard) will:
- Fix the dev/production divergence for NEW files
- **Destroy user customizations** every time the settings widget is clicked (since `ensure_settings_file` is called on every click, line 61 in `action-widgets.tsx`)
- Not fix the deeper issue: `merge_settings_to_disk()` strips comments on any backend write

**Recommendation:** Revert the always-overwrite change and implement Option A (separate template from config).
