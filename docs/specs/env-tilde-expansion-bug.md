# Bug: Tilde (`~`) Not Expanded in `cmd:env` Values

**Status:** Open
**Severity:** Medium — breaks per-agent auth isolation for any env var using `~` paths
**Date:** 2026-04-09

## Summary

Environment variables set via `cmd:env` (block metadata) are passed to spawned
processes as raw strings with no tilde expansion. When the frontend sets
`GH_CONFIG_DIR=~/.agentmux/config/gh-agentx`, the child process receives the
literal string `~/.agentmux/config/gh-agentx` instead of
`C:\Users\<user>\.agentmux\config\gh-agentx`. Tools like `gh` that don't
expand tildes themselves fail to find their config.

## Reproduction

1. Open an agent pane (any agent).
2. In the spawned shell, run: `echo $GH_CONFIG_DIR`
   - **Expected:** `/c/Users/area54/.agentmux/config/gh-agentx`
   - **Actual:** `~/.agentmux/config/gh-agentx`
3. Run `gh auth status` — fails with "not logged in" even though
   `~/.agentmux/config/gh-agentx/hosts.yml` has valid credentials.
4. Workaround: `export GH_CONFIG_DIR="$HOME/.agentmux/config/gh-agentx"` fixes it.

## Root Cause

Two sites set tilde paths without expansion:

### Site 1: Frontend (origin of the value)

**File:** `frontend/app/view/agent/agent-model.ts:200`
```typescript
envVars["GH_CONFIG_DIR"] = `~/.agentmux/config/gh-${agentSlug}`;
```

Also at lines 170 and 290 for `working_dir` and auth dir paths.

### Site 2: Backend (passes value through without expansion)

**File:** `agentmux-srv/src/backend/blockcontroller/shell.rs:527-538`
```rust
if let Some(env_map) = block_meta.get(META_KEY_CMD_ENV) {
    if let Some(obj) = env_map.as_object() {
        for (k, v) in obj {
            if let Some(val) = v.as_str() {
                // ...
                c.env(k, val);  // ← raw value, no expansion
            }
        }
    }
}
```

The backend already has `expand_home_dir()` in `base.rs:251` and uses it for
`working_dir` in `subprocess.rs:234`, but never applies it to env var values.

## Affected Paths

All tilde paths set by the frontend in `agent-model.ts`:

| Line | Variable | Value |
|------|----------|-------|
| 170 | `cmd:cwd` | `~/.agentmux/agents/<slug>` |
| 200 | `GH_CONFIG_DIR` | `~/.agentmux/config/gh-<slug>` |
| 290 | provider auth dir | `~/.agentmux/instances/v<ver>/cli/<provider>` |

Note: `cmd:cwd` may already work if the shell expands it, but env vars are
never shell-expanded.

## Fix Options

### Option A: Expand in the backend (recommended)

Apply `expand_home_dir_safe()` to every `cmd:env` value in `shell.rs` before
calling `c.env(k, val)`. This is the safest fix because:

- It uses the existing, tested `expand_home_dir` utility.
- It catches all tilde paths regardless of frontend origin.
- Env vars should always contain resolved paths — a subprocess shouldn't need
  to know the parent's home dir convention.
- `subprocess.rs` should get the same treatment for its env var injection.

```rust
// shell.rs ~line 534
let expanded = crate::backend::base::expand_home_dir_safe(val);
c.env(k, expanded.to_string_lossy().as_ref());
```

Same pattern for settings env (line 523) and subprocess env vars.

### Option B: Expand in the frontend

Replace `~` with a resolved home dir in `agent-model.ts`. Requires a frontend
API to get the home directory (e.g., `getApi().getHomeDir()`). Less reliable
because every new tilde path must remember to expand.

### Option C: Both (belt and suspenders)

Frontend sends resolved paths, backend expands as a safety net. More defensive
but adds complexity for a simple bug.

## Recommendation

**Option A.** Single fix point in the backend, covers all current and future
`cmd:env` values. Three lines of code in two files (`shell.rs` and
`subprocess.rs`).

## Files to Change

| File | Change |
|------|--------|
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | Expand tilde in env values at lines ~523 and ~534 |
| `agentmux-srv/src/backend/blockcontroller/subprocess.rs` | Expand tilde in `config.env_vars` values |
| `agentmux-srv/src/backend/base.rs` | No change — `expand_home_dir_safe` already exists |

## Testing

1. Launch an agent pane, verify `echo $GH_CONFIG_DIR` shows a fully resolved path.
2. Verify `gh auth status` works without manual export.
3. Unit test: add a case to `shell.rs` or `base.rs` confirming env values with
   `~/` prefix are expanded.
4. Verify `cmd:cwd` still works (already expanded in `subprocess.rs`, but
   confirm no double-expansion).
