# Retro: PR #70 — opener:allow-open-path Scope Regression

**Date:** 2026-03-08
**Severity:** P1 (settings button broken)
**Introduced by:** PR #70 (`agentx/clickable-links`), commit `65c1a81`
**Symptom:** Clicking the Settings widget shows "file not allowed" error. `openPath()` calls fail for all file paths.

---

## Timeline

1. **Pre-PR #70** — `opener:allow-open-path` was scoped:
   ```json
   {
     "identifier": "opener:allow-open-path",
     "allow": [{ "path": "$APPCONFIG/**" }, { "path": "$APPDATA/**" }]
   }
   ```
   Settings button worked because `ensure_settings_file` returns a path under `$APPCONFIG`.

2. **PR #70** (`65c1a81`) — Intended to "widen" access for terminal file-path clicking. Changed to unscopped:
   ```json
   "opener:allow-open-path"
   ```
   Commit message: *"Remove the scope restriction since this is a terminal app."*

3. **Result** — Settings button (and all `openPath()` calls) stopped working.

---

## Root Cause

**Misunderstanding of Tauri v2 ACL scope semantics.**

In Tauri v2's security model:
- A **permission** enables the IPC command (e.g., `open-path`)
- A **scope** defines what parameters that command can operate on (e.g., which file paths)
- If a command **requires a scope** (like `open-path` requires path validation), having the permission **without a scope** means the scope is **empty** — no inputs pass validation

So:
- `"opener:allow-open-path"` (bare string, no scope) = command enabled, **zero paths allowed**
- `{ "identifier": "opener:allow-open-path", "allow": [{ "path": "**" }] }` = command enabled, **all paths allowed**

The PR author assumed that removing the `allow` scope object meant "allow everything." In reality, it meant "allow nothing."

---

## Fix

Replace the bare string with an explicitly permissive scope:

```json
{
  "identifier": "opener:allow-open-path",
  "allow": [{ "path": "**" }]
}
```

This correctly allows `openPath()` for any file path — needed for both:
- Settings button (`$APPCONFIG/settings.json`)
- Terminal file-path clicks (arbitrary paths from terminal output)

---

## Lessons

1. **Tauri v2 ACL: unscopped ≠ unscoped.** A bare permission string enables the command but provides no scope entries. For scope-gated commands, this is effectively a deny-all.
2. **The bot-authored fix (`65c1a81`) was not manually tested.** The commit claimed to "widen" access but actually removed it entirely. A single click on the Settings widget would have caught this.
3. **Settings button should be in the smoke test checklist.** It exercises `ensure_settings_file` (Rust invoke) + `openPath` (Tauri opener plugin) — a good canary for capability regressions.

---

## Action Items

- [ ] Fix `default.json`: scope `opener:allow-open-path` with `{ "path": "**" }`
- [ ] Add settings-button click to smoke test checklist
- [ ] Document Tauri ACL scope semantics in `docs/` for future reference
