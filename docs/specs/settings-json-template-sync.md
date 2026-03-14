# Spec: Settings.json Template Sync

**Goal:** Dev and production builds show the same settings experience. In-app changes (widget reorder, transparency toggle, etc.) persist across settings file regeneration.

---

## Problem Summary

1. `ensure_settings_file` (Tauri command, called when user clicks settings widget) creates `settings.json` from `DEFAULT_SETTINGS_TEMPLATE` only if the file doesn't exist. Old files get stale templates.
2. `merge_settings_to_disk` (backend, called on any in-app setting change) round-trips through `serde_json`, destroying all JSONC comments and template structure.
3. `read_settings_raw` doesn't strip `//` comments before parsing — it strips trailing commas but not JSONC comments, so it can silently fail on template files.

Net effect: after any in-app interaction writes to settings.json, the commented template is permanently destroyed. Different builds diverge because dev's file was created before the template existed.

---

## Design

### Core Principle

`settings.json` is a **JSONC template with user overrides merged in**. Every write to this file must preserve the template structure. User values are represented as uncommented lines within the template.

### Data Flow

```
User clicks settings widget
  → ensure_settings_file()
  → reads existing user values (if file exists)
  → writes fresh template with user values merged in
  → opens in editor

App changes a setting (widget reorder, etc.)
  → merge_settings_to_disk()
  → reads existing file as JSONC, extracts user values
  → writes fresh template with ALL user values merged in
  → fs watcher detects change → broadcasts to clients
```

Both paths go through the same merge function: **template + user values → JSONC file**.

---

## Implementation

### 1. New function: `merge_into_template()` (Rust, `wconfig.rs`)

```rust
/// Merge user settings into the JSONC template.
///
/// For each user key:
///   - If the key exists as a commented line in the template (e.g., `// "key": default,`),
///     replace that line with the uncommented user value: `"key": user_value,`
///   - If the key is NOT in the template, append it before the closing `}`
///
/// Returns the merged JSONC string.
pub fn merge_into_template(
    template: &str,
    user_settings: &serde_json::Map<String, serde_json::Value>,
) -> String
```

**Line matching logic:**
- Regex for commented setting: `^\s*//\s*"([^"]+)":\s*`
- Extract key from capture group 1
- If key is in `user_settings`, emit uncommented line with user's value
- Preserve indentation from the template line

**Trailing comma handling:**
- Template lines already have trailing commas after values
- Appended lines (not in template) also get trailing commas — JSONC tolerates them, and `strip_trailing_commas` handles any that end up before `}`

**Example transform:**
```jsonc
// Input template line:
    // "window:transparent":       false,

// User has: {"window:transparent": true}
// Output:
    "window:transparent":       true,
```

For non-template keys (e.g., `widget:order`, `widget:hidden@...`):
```jsonc
    // -- Other --
    // "widget:showhelp":          true,

    // -- User Overrides --
    "widget:order": ["agent", "forge", "settings"],
    "widget:hidden@defwidget@agent": true,
}
```

### 2. Update `ensure_settings_file()` (Rust, `platform.rs`)

```rust
pub fn ensure_settings_file(app: tauri::AppHandle) -> Result<String, String> {
    let config_dir = app.path().app_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let settings_path = config_dir.join("settings.json");

    // Read existing user values (strips comments, parses JSON)
    let existing = wconfig::read_settings_raw_jsonc(&settings_path);

    // Merge user values into fresh template
    let merged = wconfig::merge_into_template(DEFAULT_SETTINGS_TEMPLATE, &existing);
    std::fs::write(&settings_path, &merged)?;

    Ok(settings_path.to_string_lossy().to_string())
}
```

### 3. Update `merge_settings_to_disk()` (Rust, `config_watcher_fs.rs`)

```rust
pub fn merge_settings_to_disk(new_keys: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    if new_keys.is_empty() {
        return Ok(());
    }
    let settings_dir = resolve_settings_dir();
    let settings_path = settings_dir.join(wconfig::SETTINGS_FILE);

    // Read ALL existing user values (comment-aware)
    let mut current = wconfig::read_settings_raw_jsonc(&settings_path);
    current.extend(new_keys);
    current.retain(|_, v| !v.is_null());

    // Write back as template + merged values (preserves comments)
    let merged = wconfig::merge_into_template(DEFAULT_SETTINGS_TEMPLATE, &current);
    std::fs::write(&settings_path, &merged)?;

    Ok(())
}
```

**Problem:** `merge_settings_to_disk` is in `agentmuxsrv-rs` (backend sidecar), but `DEFAULT_SETTINGS_TEMPLATE` is in `src-tauri` (Tauri host). Two options:

**Option A — Embed template in both crates:**
- Add `DEFAULT_SETTINGS_TEMPLATE` to `wconfig.rs` (or a shared file included via `include_str!`)
- Duplicate but simple

**Option B — Shared template file:**
- Create `settings-template.jsonc` at repo root (or `dist/schema/`)
- Both crates use `include_str!("../../settings-template.jsonc")` at compile time
- Single source of truth

**Recommended: Option B.** Create `settings-template.jsonc` in a shared location.

### 4. Fix `read_settings_raw()` (Rust, `wconfig.rs`)

Currently doesn't strip `//` comments — rename to `read_settings_raw_jsonc` and add comment stripping:

```rust
pub fn read_settings_raw_jsonc(path: &PathBuf) -> serde_json::Map<String, serde_json::Value> {
    if !path.exists() {
        return serde_json::Map::new();
    }
    match std::fs::read_to_string(path) {
        Ok(content) => {
            // Strip JSONC comments THEN trailing commas
            let stripped = json_comments::StripComments::new(content.as_bytes());
            let mut json_bytes = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::BufReader::new(stripped),
                &mut json_bytes,
            ).unwrap_or_default();
            let json_str = strip_trailing_commas(&String::from_utf8_lossy(&json_bytes));
            match serde_json::from_str::<serde_json::Value>(&json_str) {
                Ok(serde_json::Value::Object(map)) => map,
                _ => serde_json::Map::new(),
            }
        }
        Err(_) => serde_json::Map::new(),
    }
}
```

---

## File Changes

| File | Change |
|------|--------|
| `settings-template.jsonc` (NEW) | Extract template from `platform.rs`, shared by both crates |
| `src-tauri/src/commands/platform.rs` | `ensure_settings_file` → read existing + merge into template |
| `src-tauri/src/commands/platform.rs` | Remove `DEFAULT_SETTINGS_TEMPLATE` const, use `include_str!` |
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Add `merge_into_template()` function |
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Fix `read_settings_raw` → `read_settings_raw_jsonc` (strip comments) |
| `agentmuxsrv-rs/src/backend/config_watcher_fs.rs` | `merge_settings_to_disk` → use `merge_into_template` instead of `serde_json::to_string_pretty` |

---

## Edge Cases

1. **User manually edits settings.json** — next widget click or in-app change regenerates from template + their values. Manual comments they added will be lost (only the template comments survive). This is acceptable — the file header says "save to apply", not "add your own comments".

2. **New setting added to template in a future version** — automatically appears as a commented line on next open. User values from old settings that aren't in the new template get appended under "User Overrides".

3. **User sets a value back to default** — the line stays uncommented with the default value. This is fine; it's explicit.

4. **Concurrent writes** — same risk as today. The fs watcher debounces at ~300ms. No new race conditions introduced.

5. **`widget:hidden@defwidget@agent`-style keys** — these aren't in the template. They get appended in the "User Overrides" section before `}`.

---

## Testing

1. Fresh install (no settings.json) → widget click creates full template
2. Stale dev file (14 lines, no comments) → widget click regenerates template + preserves `widget:order` etc.
3. Full template file → widget click is idempotent (no diff)
4. User uncomments `"window:transparent": true` in editor → next in-app write preserves it in the template
5. In-app widget reorder → settings.json shows template + `"widget:order"` uncommented at bottom
6. `cargo test` — unit tests for `merge_into_template` with various inputs
