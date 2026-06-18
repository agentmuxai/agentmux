<!--
Copyright 2026, AgentMux Corp.
SPDX-License-Identifier: Apache-2.0
-->

# SPEC: "DEV" Badge Shows on Every Build — Runtime-Mode Self-Identification

- **Date:** 2026-06-12
- **Status:** Draft / proposed
- **Area:** `agentmux-common` (`RuntimeMode`), `agentmux-cef` (`get_is_dev`, launcher env), `frontend` status bar, macOS packaging
- **Related:** the `AGENTMUX_VITE_PORT=5289` port-collision footgun (same root cause class); `docs/analysis/ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md`

---

## 0. One-line

The status-bar **DEV** badge is meant to show **only** under `task dev` (a source-tree build). It currently shows on **portable and release builds too**, because runtime-mode self-identification trusts an **inherited env var** that a parent dev instance leaks into every descendant process.

---

## 1. Intended model (the contract)

The badge is a build-identity indicator:

| `RuntimeMode` | What it is | DEV badge |
|---|---|---|
| `Dev { branch, clone_id }` | Source-tree / `task dev` build (hot-reload) | **shown** |
| `Portable` | Extracted portable / `.app` from `task package` | hidden |
| `Installed` | Installer (dmg/msi/deb) | hidden |

A build's identity is a property of **the binary on disk** (where it lives / what marker ships beside it) — it must **not** depend on the environment of whoever launched it.

---

## 2. Current behavior — why DEV shows everywhere (code-grounded)

### The render path
- `frontend/app/statusbar/StatusBar.tsx:79-81` and `:96-98` — the badge:
  ```tsx
  <Show when={isDev()}><span class="status-version-dev">DEV</span></Show>
  ```
- `frontend/app/store/global.ts:986-989` — `isDev()` → `getApi().getIsDev()` (cached once).
- `agentmux-cef/src/ipc.rs:199` — `"get_is_dev" => commands::platform::get_is_dev()`.

### The bug — `get_is_dev()` trusts inherited env first
`agentmux-cef/src/commands/platform.rs:37-45`:
```rust
let mode = agentmux_common::RuntimeMode::from_env().or_else(|| {           // ← env FIRST
    std::env::current_exe().ok()...map(|d| agentmux_common::RuntimeMode::current(&d))
});
serde_json::json!(matches!(mode, Some(agentmux_common::RuntimeMode::Dev { .. })))
```
- `RuntimeMode::from_env()` (`agentmux-common/src/runtime_mode.rs:123-127`) reads **`AGENTMUX_RUNTIME_MODE`** and nothing else.
- The fallback `RuntimeMode::current()` (`runtime_mode.rs:69-117`) **also** honors that env at **step 1** ("Explicit `AGENTMUX_RUNTIME_MODE` override", lines 71-75) — *before* the portable-marker check (step 2) and the dev-exe-path check (step 3).

So **whoever sets `AGENTMUX_RUNTIME_MODE` wins**, over the build's real on-disk identity.

### The leak vector (verified)
A running **dev** AgentMux exports its identity into its environment, which every child/descendant process inherits. Verified inside a process spawned by the dev instance:
```
AGENTMUX_RUNTIME_MODE=dev:main
AGENTMUX_DEV=1
AGENTMUX_CLONE_ID=7bb87f0bbe471cef
```
Launch a portable or double-click-release build **as a descendant of that dev instance** (e.g. from a terminal/agent pane inside it, or `open`/exec from such a shell) and it inherits `AGENTMUX_RUNTIME_MODE=dev:main` → `from_env()` returns `Dev` → `get_is_dev()` = `true` → **DEV badge on a non-dev build.** This poisons even the portable's *own* launcher (step 1 short-circuits before the portable marker), so the wrong mode also propagates to that instance's host + srv.

> This is the **same class** as the `AGENTMUX_VITE_PORT=5289` bug: AgentMux leaks its own `AGENTMUX_*` identity vars to descendants, and descendants act on them as if they were their own.

### Secondary gap — macOS portable has no marker
`RuntimeMode::current()` step 2 calls `is_portable_marker_present()` (`runtime_mode.rs:328-336`), which looks for `agentmux-portable.marker` next to the exe **or at the `.app` bundle root** (two levels up). That marker is written by `scripts/package-portable.sh:79` — but **`scripts/package-macos.sh` never writes it.** So even with a *clean* env, a macOS portable `.app` fails the portable check and falls through to **`Installed`** (step 5), not `Portable`. (It won't show DEV — but it's misclassified, which affects anything keyed on Portable vs Installed, e.g. the instance panel / update path.)

### Why this is macOS-only — the launcher's leak-defense is gated on the wrong condition
**Both platforms ship the launcher.** On macOS it is the bundle entry point —
`AgentMux.app/Contents/MacOS/agentmux-launcher`, `CFBundleExecutable: agentmux-launcher`
(`scripts/package-macos.sh:79-86`), and it paints the native splash
(`agentmux-launcher/src/splash_mac.rs`, `#![cfg(target_os="macos")]`). The
"runs the host directly / Phase 7" caveat applies to **`task dev` only**, not to
packaged `.app`s.

The real defect: `agentmux-launcher/src/data_dir.rs:58-67`:
```rust
let is_dev = agentmux_common::is_dev_build_exe(launcher_exe_dir);   // launcher under dist/cef-dev or target/
let mode = if is_dev {
    RuntimeMode::current_path_only(launcher_exe_dir)   // ignores inherited env
} else {
    RuntimeMode::current(launcher_exe_dir)             // HONORS inherited AGENTMUX_RUNTIME_MODE (step 1)
};
```
The env-ignoring `current_path_only()` is used **only when the launcher binary is
itself a dev build.** A **packaged** launcher (`.app`/portable) takes the `else`
branch → `current()` → **step 1 trusts a leaked `AGENTMUX_RUNTIME_MODE`**. So a
packaged build launched as a **descendant of a dev instance** (from a terminal/agent
pane inside it — exactly how builds get launched while developing) inherits
`dev:main`, and the launcher propagates `dev:main` to the host → **DEV**. The guard
is applied to the case that *doesn't* leak across instances (a dev launcher) and
skipped for the case that *does* (a packaged build started inside a dev instance).
The comment at `data_dir.rs:60-61` even names the leak ("Prevents inheriting
`AGENTMUX_RUNTIME_MODE` from a parent AgentMux") — it just guards the wrong branch.

In practice:
- **Windows** portable carries the **marker** (`package-portable.sh:79`) and is run
  **clean** (Explorer double-click, no inherited env) → `current()` step 2 →
  `Portable` → no DEV.
- **macOS** `.app` carries **no marker** (`package-macos.sh` gap) *and* is launched
  from inside the dev environment → `current()` step 1 honors the inherited
  `dev:main` → DEV. (A genuinely clean Finder double-click would fall to `Installed`
  — also no DEV — so the badge appears because of the **inherited env**, not the path.)

`launchctl getenv` confirms `AGENTMUX_*` are **not** set session-wide on macOS, so
the leak is process inheritance (descendant of the dev instance), not a global export.

### Divergent copies
Three independent implementations of the same "am I dev?" check exist, all with the env-first precedence:
- `agentmux-cef/src/commands/platform.rs:37` (`get_is_dev` — the badge)
- `agentmux-cef/src/macos_menu.rs:180-187` (`is_dev()` → `"AgentMux DEV"` app/menu name, line 200)
- `agentmux-cef/src/app.rs:712-718` (window/runtime-style `is_dev`)

They can (and will) drift.

---

## 3. Goals / Non-goals

**Goals**
1. The DEV badge reflects the **binary's** identity, immune to a leaked parent env.
2. A macOS portable `.app` identifies as `Portable`, not `Installed`.
3. One shared dev-check, not three.
4. Stop AgentMux from leaking its `AGENTMUX_*` identity vars to unrelated descendants (fixes the badge **and** the vite-port collision).

**Non-goals**
- Changing how the launcher passes mode to its *own* host/srv for **data-dir** resolution (that legitimately uses `from_env()` to avoid re-detection desync — see `from_env` doc, `runtime_mode.rs:119-122`). We only change **self-identification of build type**, not intra-instance plumbing.
- The `AGENTMUX_RUNTIME_MODE` **explicit** override for tests/CI/ops stays supported.

---

## 4. Design / fix

### Fix A — self-identify by path, not inherited env (primary)
For build-identity questions (the DEV badge), use the env-ignoring detector the code **already provides**:

`runtime_mode.rs:129-144` — `current_path_only()`: *"Use when the env can't be trusted (e.g., the binary was launched as a child of a different AgentMux process that set its own `AGENTMUX_*` vars)."* Precedence: portable-marker → dev-exe-path → installed.

Change `get_is_dev()` to:
```rust
pub fn get_is_dev() -> serde_json::Value {
    let is_dev = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|d| matches!(agentmux_common::RuntimeMode::current_path_only(&d),
                          agentmux_common::RuntimeMode::Dev { .. }))
        .unwrap_or(false);
    serde_json::json!(is_dev)
}
```
Now a `task dev` host (`dist/cef-dev/agentmux-cef`) → `Dev` (badge shown); a portable `.app` / installed host → `Portable`/`Installed` (badge hidden) — regardless of any inherited `AGENTMUX_RUNTIME_MODE`.

### Fix B — write the portable marker on macOS
`scripts/package-macos.sh`: after staging `AgentMux.app`, write the marker at the bundle root (mirrors `package-portable.sh:79`, and matches the bundle-root probe in `is_portable_marker_present`):
```sh
printf 'AgentMux portable build %s\n' "$LABEL" > "$APP/../agentmux-portable.marker"   # bundle-root, beside AgentMux.app
```
(Confirm placement against `is_portable_marker_present`'s two-levels-up probe for `<Bundle>.app/Contents/MacOS/<exe>`.)

### Fix C — launcher: don't gate the leak-defense on `is_dev_build_exe` (the root cause)
`agentmux-launcher/src/data_dir.rs:58-85` currently uses the env-ignoring
`current_path_only()` **only** for dev-build launchers; packaged launchers fall to
`current()` and trust a leaked `AGENTMUX_RUNTIME_MODE`. The launcher is the
process-tree root — its identity is its **own exe path + marker**, never an inherited
env — so it should use path-only **for all builds**:
```rust
let is_dev = agentmux_common::is_dev_build_exe(launcher_exe_dir);
let mode = if is_dev { current_path_only(dir) } else { current(dir) };  // before
// after:
let mode = RuntimeMode::current_path_only(launcher_exe_dir);
let common = CommonDataPaths::resolve_path_only(version, &mode)?;       // already the dev branch (line 81-85) — make it unconditional
```
An **explicit** operator/test override should move to a *distinct* variable
(e.g. `AGENTMUX_RUNTIME_MODE_FORCE`) so "intentional override" can never be confused
with "leaked inheritance." This corrects the mode for the **whole instance** (data
dir, update path, badge), packaged builds included, and — being the same inherited-
`AGENTMUX_*` class — also closes the `AGENTMUX_VITE_PORT` collision (§8).

### Fix D — one helper
Collapse `platform.rs::get_is_dev`, `macos_menu.rs::is_dev`, `app.rs` is_dev into a single `agentmux_common` (or host) helper built on `current_path_only()`. All three then agree by construction.

---

## 5. Touch points

| Fix | File | Change |
|---|---|---|
| A | `agentmux-cef/src/commands/platform.rs:37-45` | `get_is_dev()` → `current_path_only(exe_dir)`, drop `from_env()` precedence |
| B | `scripts/package-macos.sh` | write `agentmux-portable.marker` at bundle root |
| C | `agentmux-launcher/src/data_dir.rs:58-85` | use `current_path_only()` + `resolve_path_only()` **unconditionally** (drop the `is_dev_build_exe` gate); move explicit override to a distinct `AGENTMUX_RUNTIME_MODE_FORCE` |
| D | `agentmux-cef/src/macos_menu.rs:180`, `app.rs:712`, `platform.rs:37` | route through one shared path-based helper |
| — | `agentmux-common/src/runtime_mode.rs:69-117` | (optional) document that env step 1 is for explicit override only; not a substitute for path identity |

---

## 6. Testing
- **Unit (`runtime_mode`):** `current_path_only()` with a stubbed exe dir → `Dev` for a dev-build path, `Portable` when the marker is present, `Installed` otherwise — **with `AGENTMUX_RUNTIME_MODE=dev:main` set in env** (must be ignored).
- **Regression:** assert `get_is_dev()` returns `false` for a portable/installed exe path even when `AGENTMUX_RUNTIME_MODE=dev:*` is exported (reproduces the leak).
- **macOS packaging:** after `task package:macos`, assert `agentmux-portable.marker` exists at the bundle root and `is_portable_marker_present` finds it.
- **Manual:** launch the built `.app` **from a terminal inside the dev instance** (worst case for leakage) → no DEV badge; `task dev` → DEV badge present.

---

## 7. Rollout / phasing
- **P1 — kill the badge symptom:** Fix A (`get_is_dev` → `current_path_only`, one function) + Fix B (macOS marker, one line). Correct on all builds; no launcher changes. **NB:** Fix A corrects only the *badge* — the launcher still mis-resolves the **data dir** for a leaked-env packaged build until Fix C lands.
- **P2 — fix the root in the launcher:** Fix C (`data_dir.rs` path-only, unconditionally). Corrects the *whole* instance (data dir + update path + badge) for packaged builds, and closes the `AGENTMUX_*`-leak family incl. the vite-port collision (§8).
- **P3 — consolidate:** Fix D (one helper) so the menu name (`AgentMux DEV`) and the badge can't diverge.

---

## 8. Relationship to the `AGENTMUX_VITE_PORT` collision
Same disease, different symptom. The dev instance exports `AGENTMUX_VITE_PORT` *and* `AGENTMUX_RUNTIME_MODE`/`AGENTMUX_DEV`/`AGENTMUX_CLONE_ID`; descendants inherit and act on them:
- `AGENTMUX_VITE_PORT` → every nested `task dev` tries the parent's port (5289) → "port already in use".
- `AGENTMUX_RUNTIME_MODE` → every nested build self-reports `Dev` → DEV badge on portable/release.

**Fix C handles both.** Fix A is the badge-specific guard that's worth shipping immediately regardless.

---

## 9. Open questions
1. Should `current()` step 1 (env override) be demoted **below** the portable marker, so a real portable can't be flipped to Dev by a stray env? (Tests/CI rely on the override — a dedicated `AGENTMUX_RUNTIME_MODE_FORCE` would separate "explicit override" from "leaked inheritance".)
2. ~~Does `exe_dir_is_dev_build()` recognize the macOS `task dev` host path so Fix A keeps the badge under `task dev`?~~ **Resolved:** yes — `runtime_mode.rs:358` matches `parent=="dist" && name=="cef-dev"`, and macOS `task dev` runs the host from `dist/cef-dev/`. Under Fix A: `task dev` → `Dev` (badge shown), packaged `.app` → not-dev path → `Installed` (badge hidden). Verified correct on both platforms.
3. Scrub-list scope for Fix C — exactly which `AGENTMUX_*` vars are "instance identity" (must not inherit) vs legitimately inheritable?
