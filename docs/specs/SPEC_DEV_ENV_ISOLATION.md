# Dev-Build Environment Isolation

**Status:** Implemented (core); UX gaps documented below  
**Date:** 2026-06-24  
**Owner:** AgentX  
**Referenced by:** `agentmux-launcher/src/data_dir.rs` ("see SPEC_DEV_ENV_ISOLATION")

---

## 1. Problem statement

When a developer runs `task dev` or `task dev:local` while the stable .app is
already running, several things can go wrong:

1. The dev build silently adopts the stable instance's data directory and
   process-singleton socket, forwarding its open request into the wrong window.
2. The macOS Dock shows two "AgentMux" entries with no visible distinction
   between stable and dev — the developer clicks the wrong tile and wonders
   why their code change has no effect.
3. A plain `task dev` (no `:local` bump) while the SAME version's stable
   instance is running always says "already running" and never launches.

---

## 2. Isolation stack

Isolation is enforced at four independent layers. All four must agree for a
dev instance to be truly separate.

### 2.1 Data directory

**Mechanism:** `RuntimeMode` → `DataPaths::resolve_path_only`  
**Files:** `agentmux-common/src/runtime_mode.rs`, `agentmux-common/src/data_paths.rs`

Resolution priority (path-only branch, used by all dev builds):

| Priority | Condition | Result |
|----------|-----------|--------|
| 1 | `agentmux-portable.marker` present | `RuntimeMode::Portable` → `~/.agentmux/versions/<ver>/` |
| 2 | exe under `dist/cef-dev*/`, `target/debug/`, `target/release/` | `RuntimeMode::Dev { branch, clone_id }` → `~/.agentmux/dev/<branch>/<clone_id>/` |
| 3 | default | `RuntimeMode::Installed` → `~/.agentmux/channels/<AGENTMUX_BUILD_CHANNEL_DEFAULT>/versions/<ver>/` |

`clone_id` is a 16-hex-char hash of the repo root path, making two checkouts
of the same branch on the same machine use separate directories.

**Confirmed (2026-06-24):** `task dev:local` resolves to  
`~/.agentmux/dev/fix-browser-pane-black-freeze-macos/50e2149139f457df/`  
— confirmed by inspecting the live env in `AGENTMUX_DATA_DIR`.

### 2.2 Process-singleton socket

**Mechanism:** `hash::data_dir_hash16(data_dir, pipe_version)` → socket path  
**Files:** `agentmux-launcher/src/hash.rs:54-60`, `agentmux-launcher/src/main.rs:814-816`

```rust
let pipe_version = option_env!("AGENTMUX_BUILD_LABEL")
    .unwrap_or(env!("CARGO_PKG_VERSION"));
let dir_hash = hash::data_dir_hash16(&paths.data_dir, pipe_version);
```

`pipe_version` = `AGENTMUX_BUILD_LABEL` at compile time, or `CARGO_PKG_VERSION`
as fallback. The hash is FNV-1a-64 over `canonical_data_dir\x00pipe_version`,
hex-encoded to 16 chars.

Because both `data_dir` and `pipe_version` differ between stable and dev, their
sockets are distinct. Two instances can coexist without conflict.

**Collision case:** `task dev` (no `:local`) launched while the SAME version's
stable instance is running. Both resolve to identical `data_dir` + `pipe_version`
→ identical hash → "AgentMux is already running for this data directory."
This is correct single-instance behavior — see §4 for the fix.

### 2.3 macOS bundle identifier

**Mechanism:** `CFBundleIdentifier = ai.agentmux.<channel>.<version>`  
**Spec:** `docs/specs/SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md §4`  
**Status:** Implemented 2026-06-21

Format: `ai.agentmux.<channel>.<version>`, e.g.:
- Stable 0.49.1 → `ai.agentmux.stable.0.49.1`
- Dev:local 0.49.2, branch `fix-browser-pane-black-freeze-macos`, clone `50e2149139f457df` →  
  `ai.agentmux.dev-fix-browser-pane-black-freeze-macos-50e2149139f457df.0.49.2`

LaunchServices treats these as distinct applications. Double-clicking the
stable .app while the dev instance is running launches a new stable window
rather than reactivating the dev window. No `open -n` required.

### 2.4 Inherited env suppression (pane sentinel)

**Mechanism:** `AGENTMUX` sentinel → `ignore_ambient=true` → path-only resolution  
**File:** `agentmux-launcher/src/data_dir.rs:58-105`

```rust
let is_dev = agentmux_common::is_dev_build_exe(launcher_exe_dir);
let nested = std::env::var_os("AGENTMUX").is_some();
let ignore_ambient = is_dev || nested;

let mode = if ignore_ambient {
    RuntimeMode::current_path_only(launcher_exe_dir)  // ignores inherited AGENTMUX_RUNTIME_MODE
} else {
    RuntimeMode::current(launcher_exe_dir)
};
let common = if ignore_ambient {
    CommonDataPaths::resolve_path_only(version, &mode)?  // ignores inherited AGENTMUX_CHANNEL
} else {
    CommonDataPaths::resolve(version, &mode)?
};
```

`agentmux-srv` sets `AGENTMUX=1` for every pane shell it spawns. So a dev
build launched from inside an existing pane (an agent running `task dev`)
always hits `ignore_ambient=true`, drops all inherited `AGENTMUX_*` vars,
and resolves paths purely from the exe's location on disk.

Without this guard, a pane shell running inside a stable instance would pass
`AGENTMUX_RUNTIME_MODE=installed` (or `AGENTMUX_CHANNEL=stable`) into the dev
launcher, routing it into the stable data dir and hitting the process-singleton
("already running"). See `docs/specs/dev-build-env-isolation.md` for the
original failure analysis (PR #719).

---

## 3. Isolation matrix

| Scenario | Data dir | Socket | Bundle ID | Result |
|----------|----------|--------|-----------|--------|
| Stable .app (0.49.1) | `channels/stable/versions/0.49.1/` | hash-A | `ai.agentmux.stable.0.49.1` | Runs |
| `task dev:local` (0.49.2, branch X, clone Y) | `dev/X/Y/` | hash-B | `ai.agentmux.dev-X-Y.0.49.2` | Isolated ✅ |
| `task dev` (same 0.49.1 as .app, from pane) | `dev/X/Y/` | hash-C | `ai.agentmux.dev-X-Y.0.49.1` | Isolated ✅ |
| `task dev` (same 0.49.1 as .app, from external shell, no stable running) | `dev/X/Y/` | hash-C | `ai.agentmux.dev-X-Y.0.49.1` | Isolated ✅ |
| `task dev` (same 0.49.1 as .app, from external shell, stable already running) | `dev/X/Y/` | hash-C | Different bundle ID | Isolated ✅ — independent windows |
| `task dev` (0.49.1), SECOND `task dev` call while first still building | `dev/X/Y/` | hash-C (same) | Same | "Already running" → correct single-instance ✅ |

---

## 4. Known gap: `task dev` without `:local` is misleading on first run

`task dev` (no `:local`) builds and launches using the version from `Cargo.toml`
— the same version as the released stable .app when no bump has occurred.
A user running it for the first time expects a dev window. What they see instead
depends on:

- **If no other instance is running:** dev launches fine.
- **If a dev instance from a previous `task dev` run is already running:**  
  "AgentMux is already running" → focus existing dev window. Correct but confusing.
- **If the stable .app of the SAME version is running:**  
  Data dirs differ (dev vs channels/stable), so isolated at the data layer.
  But the Taskfile output "AgentMux is already running" appears if there is
  a stale dev socket from a prior run — user must run `task dev:local` instead.

**Recommendation:** Make `task dev`'s dev:serve step proactively wipe the dev
socket dir (or use a timestamp subdir like Windows does) so stale sockets never
cause false "already running" messages. Alternatively, always prefer `task dev:local`
for interactive dev sessions and document `task dev` as CI/automation-only.

---

## 5. Known gap: Dock / window-title confusion between running instances

When both a stable and a dev:local instance are running simultaneously, both
appear in the Dock as "AgentMux" with the standard icon. There is no visual
channel or version indicator. A developer clicking the wrong Dock tile focuses
the wrong instance and may not notice.

The `applicationShouldHandleReopen:` handler in `agentmux-cef/src/macos_menu.rs`
fires on the process that **owns the Dock tile**. With per-version bundle IDs,
this is unambiguous — each instance owns its own tile. But the tiles look
identical.

**Recommendations (in priority order):**

1. **Window title suffix:** Append `[dev:<branch>]` to the main window title
   for non-stable channels (read from `AGENTMUX_CHANNEL`). Visible in Cmd+Tab
   and the Window menu. Zero Dock-icon changes required.

2. **Dev-channel Dock badge or icon overlay:** Use `NSDockTile` to overlay a
   small "DEV" badge on the Dock icon when `AGENTMUX_RUNTIME_MODE` starts with
   `dev:`. Implemented in the CEF host at the same call site as the existing
   `NSApplicationActivationPolicyRegular` setup (`agentmux-cef/src/lib.rs:892`).

3. **Menubar channel indicator:** Show `AgentMux [dev]` in the macOS menu bar
   app name (already possible via `setApplicationName` on `NSRunningApplication`).

Only #1 is zero-risk; #2 and #3 touch AppKit APIs that interact with CEF's
`NSApplication` subclass and need care around the macOS 26 Tahoe compat shims
already in place (`lib.rs:1047-1158`).

---

## 6. Verification checklist

After any change to the isolation stack, verify:

```
[ ] task dev:local from inside stable pane:
      AGENTMUX_DATA_DIR must be under ~/.agentmux/dev/<branch>/<clone_id>/
      NOT under ~/.agentmux/channels/stable/

[ ] task dev (no bump) from inside stable pane while stable is running:
      A separate dev window opens (different bundle ID, different data_dir)
      The stable window is NOT focused or disturbed

[ ] Double-click stable .app while dev:local is running:
      A NEW stable window opens (not the dev window)
      Verified by checking the new window's AGENTMUX_CHANNEL env

[ ] task dev from inside dev:local pane (nested dev):
      nested=true via AGENTMUX sentinel
      inner dev resolves to SAME dev data dir as outer (correct — same branch/clone)
      Single-instance "already running" — inner exits, outer window focused

[ ] Two checkouts of same branch:
      clone_id differs (hash of repo root)
      Both can run simultaneously with separate data dirs and sockets

[ ] Window title shows channel for non-stable instances (once #1 above is implemented)
```

---

## 7. Related specs and PRs

| Document | Coverage |
|----------|----------|
| `docs/specs/dev-build-env-isolation.md` | Original fix for nested-pane env inheritance (PR #719) |
| `docs/specs/SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md` | Per-version CFBundleIdentifier and reopen handler |
| `docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` | Multi-instance socket hardening |
| `docs/specs/SPEC_VERSION_ISOLATION_2026_06_01.md` | Version-scoped data dirs |
| `agentmux-launcher/src/data_dir.rs` | `resolve_paths` — the single authoritative isolation call site |
| `agentmux-launcher/src/hash.rs` | `data_dir_hash16` — socket hash function |
| `agentmux-launcher/src/splash_mac.rs` | `set_reopen_target` / `should_handle_reopen` |
| `agentmux-cef/src/macos_menu.rs` | `install_reopen_handler` (host-side) |
