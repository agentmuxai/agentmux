# Spec: Stash Reapply — Conflict Analysis

**Stash:** `stash@{0}` — WIP on main: `14179b33`
**Current HEAD:** `3bd48934` (feat: log consolidation — unified dir, env var, pointer files)

Our stash contains: the macOS CEF build implementation (`cef:bundle:darwin`, `package:macos` in Taskfile.yml) and the macOS resource path fix in `main.rs`.

---

## Files in Stash

| File | Lines changed | Conflict risk |
|------|--------------|---------------|
| `Taskfile.yml` | +112 lines | **None** |
| `agentmux-cef/src/main.rs` | +17 lines | **Low — likely auto-merges** |
| `package-lock.json` | −136 lines | **High — will conflict, but trivially resolved** |

---

## 1. `Taskfile.yml` — No Conflict

`git diff 14179b33..HEAD -- Taskfile.yml` produces zero output. Upstream made **no changes** to Taskfile.yml in the 17 commits between the stash base and HEAD. Our additions apply cleanly:

- `package:macos` (line 73): stub → full `.app` + DMG implementation
- `cef:bundle:darwin` (line 475): stub → full framework + sidecar + wsh + frontend copy

No manual resolution needed.

---

## 2. `agentmux-cef/src/main.rs` — Low Risk, Likely Auto-merges

### What we changed (stash)

At lines 221-222 in stash base (now lines **216-217** in HEAD), replaced:

```rust
let resources_dir = CefString::from(base_dir.to_str().unwrap_or(""));
let locales_dir = CefString::from(base_dir.join("locales").to_str().unwrap_or(""));
```

with a macOS conditional block (17 lines):

```rust
// On macOS, pak files and locale paks live inside the CEF framework's
// Resources/ directory — not alongside the executable.
#[cfg(target_os = "macos")]
let (resources_dir, locales_dir) = {
    let fw_resources = exe_dir
        .join("../Frameworks/Chromium Embedded Framework.framework/Resources");
    let s = fw_resources.to_str().unwrap_or("");
    (CefString::from(s), CefString::from(s))
};
#[cfg(not(target_os = "macos"))]
let (resources_dir, locales_dir) = (
    CefString::from(base_dir.to_str().unwrap_or("")),
    CefString::from(base_dir.join("locales").to_str().unwrap_or("")),
);
```

### What upstream changed (14179b33 → 3bd48934)

Upstream touched **different regions** of the file:

| Region | Change |
|--------|--------|
| Line 26 (mod declarations) | Added `mod memory_heartbeat;` |
| Lines 54-68 | **Removed** early tracing init (moved to `init_logging()`) |
| Line 72 | `tracing::error!` → `eprintln!` (pre-init, subprocess) |
| Line 94 | `tracing::info!` → `eprintln!` (subprocess exit) |
| Line 103 | Added `let _log_guard = init_logging();` + comment |
| Line 252 | Added `memory_heartbeat::start();` + comment |
| Lines 289-349 | Added `fn init_logging()` function (+61 lines) |

**Critical fact**: Lines 216-217 (current HEAD) — `let resources_dir = ...` and `let locales_dir = ...` — were **not touched by upstream**. They are identical in the stash base and HEAD.

### Merge outcome

Git 3-way merge comparison:
- **Common ancestor (14179b33)**: lines 221-222 = the two `let` lines
- **HEAD (3bd48934)**: same two lines, just shifted to 216-217 (upstream removed 13 lines of early tracing above this section, added 8 lines in other sections)
- **Stash**: replaces those 2 lines with 17-line conditional block

Since the upstream didn't touch those specific lines, git should auto-merge cleanly. **If a conflict does appear**, it will be a false positive from hunk adjacency — resolved by manually applying the `#[cfg(target_os = "macos")]` block at the current lines 216-217 in HEAD.

---

## 3. `package-lock.json` — Will Conflict, Trivially Resolved

### Why it conflicts

- **Upstream** bumped `version` from `0.33.42` → `0.33.60` (17 version bumps in those commits), affecting the top of the file and throughout
- **Stash** removed `"dev": true` flags from several optional npm package entries — a side effect of running `npm install` during our session (not intentional changes)

Both touch overlapping regions → git will flag conflicts.

### Resolution

**Discard stash's package-lock.json entirely.** The `"dev": true` removal was a side effect, not intentional. HEAD's version is authoritative.

```bash
git checkout HEAD -- package-lock.json
```

---

## Reapply Plan

```bash
# 1. Pop stash — expect clean apply for Taskfile.yml
#    main.rs may auto-merge or need manual fix at lines 216-217
git stash pop

# 2. Resolve package-lock.json — always discard stash version
git checkout HEAD -- package-lock.json

# 3. If main.rs has conflicts, manually apply the macOS #[cfg] block at lines 216-217.
#    The block to insert is documented in the "What we changed" section above.

# 4. Verify
git diff --stat HEAD
```

### Smoke test after reapply

```bash
task cef:build       # must succeed first
task cef:bundle      # exercises our cef:bundle:darwin implementation
cd dist/cef && ./agentmux-cef --url=http://localhost:5173
```

Expected log output (no errors):
- `"AgentMux CEF host starting"` — init_logging() fired
- `"Backend ready: ws=..."` — sidecar found and spawned
- No `"Failed to load CEF framework"` — library_loader resolved Frameworks/

---

## Risk Summary

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| main.rs auto-merges with false conflict | Low | Low | Manually insert #[cfg] block at lines 216-217 |
| Taskfile.yml conflict | None | — | — |
| package-lock.json conflict | High | None | `git checkout HEAD -- package-lock.json` |
| Our macOS fix is wrong for new upstream structure | Low | High | Smoke test: run `./agentmux-cef` and check for framework load error |
| `init_logging()` + our `#[cfg]` interact badly | None | — | They are in separate scopes; no interaction |
