# Spec: Open Settings in Code Editor

## Problem

Clicking "Settings" opens `settings.json` in whichever application the OS associates with `.json` files. On macOS this is MaxMSP (or any other app the user has set as default for `.json`). The intent is to open in a code editor.

## Root Cause

**Flow today:**

```
action-widgets.tsx (handleWidgetSelect)
  → invoke("ensure_settings_file")          // Rust: create/return path to settings.json
  → getApi().openNativePath(path)            // tauri-api.ts
  → openPath(path)                           // @tauri-apps/plugin-opener
  → macOS: open /path/to/settings.json      // OS file association → MaxMSP
```

The app has **no editor detection logic**. It does not check `$EDITOR`, `$VISUAL`, PATH, or known editor locations. It fully delegates to the OS native file association, which on macOS respects the user's `.json` handler regardless of whether it is a code editor.

**Relevant code locations:**

| File | Location | Role |
|------|----------|------|
| `frontend/app/window/action-widgets.tsx` | ~line 31 `handleWidgetSelect` | Triggers the open |
| `frontend/util/tauri-api.ts` | `openNativePath` | Calls `openPath()` from plugin-opener |
| `src-tauri/src/commands/platform.rs` | `ensure_settings_file` | Creates file, returns path |
| Tauri plugin-opener | Cargo.toml `tauri-plugin-opener = "=2.5"` | Delegates to OS `open` |

## Proposed Fix

Replace the blind `openNativePath` call with a new Tauri command `open_in_editor` that:

1. **Checks `$EDITOR` / `$VISUAL`** environment variables first (terminal user preference)
2. **Probes known code editors** in priority order:
   - `code` (VSCode)
   - `cursor` (Cursor)
   - `zed` (Zed)
   - `subl` (Sublime Text)
   - `atom`
   - `/Applications/Visual Studio Code.app`
   - `/Applications/Cursor.app`
   - `/Applications/Zed.app`
   - `/Applications/Sublime Text.app`
3. **Falls back to `openPath`** (OS default) only if nothing is found

### macOS specifics

On macOS, app bundles in `/Applications` are detectable even if not on PATH. Use `open -a "App Name"` or directly invoke the binary inside the bundle (`/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code`).

### Backend command (Rust)

Add to `src-tauri/src/commands/platform.rs`:

```rust
/// Open a file in the best available code editor.
/// Priority: $EDITOR → $VISUAL → known editors by PATH → macOS .app bundles → OS default.
#[tauri::command]
pub fn open_in_editor(path: String) -> Result<(), String> {
    // 1. $EDITOR / $VISUAL
    for var in &["EDITOR", "VISUAL"] {
        if let Ok(editor) = std::env::var(var) {
            let editor = editor.trim().to_string();
            if !editor.is_empty() {
                if std::process::Command::new(&editor).arg(&path).spawn().is_ok() {
                    return Ok(());
                }
            }
        }
    }

    // 2. CLI editors on PATH
    let cli_editors = ["code", "cursor", "zed", "subl", "atom", "hx", "nvim", "vim"];
    for editor in &cli_editors {
        if std::process::Command::new(editor).arg(&path).spawn().is_ok() {
            return Ok(());
        }
    }

    // 3. macOS .app bundles
    #[cfg(target_os = "macos")]
    {
        let app_bundles = [
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
            "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
            "/Applications/Zed.app/Contents/MacOS/zed",
            "/Applications/Sublime Text.app/Contents/SharedSupport/bin/subl",
        ];
        for bin in &app_bundles {
            if std::path::Path::new(bin).exists() {
                if std::process::Command::new(bin).arg(&path).spawn().is_ok() {
                    return Ok(());
                }
            }
        }
    }

    // 4. OS default fallback
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &path])
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}
```

Register in `src-tauri/src/lib.rs` under `.invoke_handler(tauri::generate_handler![...])`.

### Frontend change

In `frontend/app/window/action-widgets.tsx`, replace:

```ts
// Before
getApi().openNativePath(path);

// After
await invoke("open_in_editor", { path });
```

Or surface it through `tauri-api.ts` as `openInEditor(path)` if preferred.

## Out of Scope

- Letting users configure their preferred editor in `settings.json` itself (circular problem)
- Terminal-mode editors (nano, vim) opened inside a pane — these are CLI-only and spawn headlessly
- Windows app detection beyond PATH

## Acceptance Criteria

- [ ] Clicking Settings on macOS with VSCode installed opens `settings.json` in VSCode
- [ ] Clicking Settings on macOS with Cursor installed (and no VSCode) opens in Cursor
- [ ] If `$EDITOR=nano` is set, it is ignored in favor of GUI editors (or document that `$EDITOR` wins)
- [ ] If no known editor found, falls back to OS default (same behavior as today)
- [ ] Works on Windows (PATH-based detection) and Linux (PATH + xdg-open fallback)
