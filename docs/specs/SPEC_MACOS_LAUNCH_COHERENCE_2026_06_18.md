# SPEC: macOS Launch Coherence — Channel-Scoped Bundle ID, Reopen Handler, Unix Window Forward

- **Date:** 2026-06-18
- **Status:** Proposed → implementation in progress
- **Owner decision:** bundle id follows the channel; the default/release channel `stable` is suffixed explicitly (`ai.agentmux.cef.stable`).

---

## 1. Symptoms (observed testing release DMGs side by side)

While running 0.44.1 / 0.45.0 / 0.46.1 / 0.46.3 simultaneously on macOS:

1. **"You can't open AgentMux because it is not responding"** when double-clicking a build while a *different* version is already running.
2. **Double-clicking a running build opens no second window** (expected: relaunch → new window, the Windows behavior).
3. **Version leakage on disk** — dozens of stale `ai.agentmux.app.v*`, `ai.agentmux.cef.v*`, `com.agentmuxhq.*` dirs in `~/Library/Application Support` and `~/Library/Caches`.

---

## 2. Root causes

### A. One constant bundle identifier → LaunchServices collision
`scripts/package-macos.sh:58` → `BUNDLE_ID="ai.agentmux.cef"`, constant across every version **and** channel (helpers at `:171`, main app at `:207`). macOS LaunchServices keys app identity on `CFBundleIdentifier`. With one id for all builds, double-clicking build B while build A runs makes LaunchServices **reactivate A** (send it the reopen Apple Event) instead of launching B.

### B. No reopen handler
No `applicationShouldHandleReopen:` / `kAEReopenApplication` handler exists in `agentmux-cef` or `agentmux-launcher` (only `setActivationPolicy`, `agentmux-cef/src/main.rs:1419-1461`). So the reactivation from (A) has nothing to answer it → LaunchServices times out → **"not responding."** A normal double-click of the running app also produces no new window.

### C. Unix `open_new_window` forward is not wired (Windows-only)
The relaunch → new-window forward **is implemented, but only on Windows**:
- **Unix second-instance path:** `agentmux-launcher/src/main.rs:585-597` detects the running instance via the unix socket and `std::process::exit(0)`. Comment: *"Phase-2 arg-forwarding is a separate PR."* No forward.
- **Windows path:** `main.rs:1292` calls `forward_open_new_window(&paths.data_dir, &dir_hash)`.
- **The forward itself is platform-agnostic:** `forward_open_new_window` (`main.rs:1935`) reads `ipc-port-<dir_hash>` and HTTP-POSTs `{"cmd":"open_new_window"}` to `127.0.0.1:<port>/ipc`. No Windows APIs.
- **Host endpoint is cross-platform:** `agentmux-cef/src/ipc.rs:225` → `"open_new_window" => commands::window::open_new_window(state)`.
- **In-app affordance already uses it:** `frontend/app/statusbar/InstancePanel.tsx:527` ("+ Open another window") → same endpoint. (This is the current macOS workaround.)

### D. Disk leakage — wipe script is Windows-only
`scripts/wipe-old-data-dirs.sh` shells out to `powershell.exe`; it has never run on macOS. **Out of scope here** (tracked separately).

---

## 3. The two attach layers (must agree)

A "new window" / relaunch resolves to a running instance at two independent layers:

- **OS layer — `CFBundleIdentifier`:** LaunchServices decides which *process* a double-click / reopen routes to.
- **Launcher layer — data dir:** instance key is `channels/<channel>/versions/<version>/`; `runtime/ipc-port` lives there (`agentmux-common/src/data_paths.rs:192-202`); the single-instance socket and the `open_new_window` forward both attach by this key.

Today the OS layer is a **constant** while the launcher layer is **per-(channel, version)**. They disagree, so the OS can route a double-click to the wrong-channel (or a hung) process — the source of symptoms 1 & 2.

---

## 4. Decision: bundle id follows the channel

`CFBundleIdentifier = ai.agentmux.cef.<channel>`, derived from the **same** channel the binary is compiled with — `AGENTMUX_BUILD_CHANNEL_DEFAULT` (compile-time `option_env!`, default `"stable"`, `data_paths.rs:55-57`; `"default"` is a synonym for `"stable"`, `:458`). `scripts/package.sh:151` already exports `$CHANNEL` into `AGENTMUX_BUILD_CHANNEL_DEFAULT` for the cargo build; `package-macos.sh` reads the same value for the Info.plist. **Single source of truth** → the OS identity and the runtime channel cannot drift apart.

- **Default/release channel `stable` → `ai.agentmux.cef.stable`** (explicit suffix — uniform scheme, no bare special-case).
- `dev` → `ai.agentmux.cef.dev`.
- local builds → `ai.agentmux.cef.local-<branch>-<hash>-<build-id>` (sanitized to the bundle-id charset: `[A-Za-z0-9-.]`).
- Helpers inherit the suffix automatically via `${BUNDLE_ID}.helper[.type]` (`package-macos.sh:171`).

### What this fixes / doesn't
- **Cross-channel:** dev/local builds become **distinct macOS apps** → double-click-launchable side by side with a running stable, no `open -n`. Local builds already carry a per-build hash *in the channel*, so every local build is its own app for free.
- **Same channel, different versions** (stable 0.45.0 vs 0.46.3): still share `ai.agentmux.cef.stable` **by design**. Running two stable *releases* at once stays an `open -n` operation — intentional (avoids per-release Dock-app sprawl, which is what created the data-dir explosion in the first place).

### Migration consequence of `.stable`
Existing stable installs report `ai.agentmux.cef` today. Switching to `ai.agentmux.cef.stable` is a **one-time app-identity change**: on the next stable update macOS treats it as a new app → previously-granted permissions (notifications, screen recording, accessibility), login items, and default-app associations reset **once**; LaunchServices may keep a duplicate registration until the stale one is pruned. Accepted by the owner in exchange for a uniform, drift-proof scheme.

---

## 5. Fix plan (one branch / PR)

1. **`scripts/package-macos.sh`** — derive `BUNDLE_ID="ai.agentmux.cef.${CHANNEL}"` from `${AGENTMUX_BUILD_CHANNEL_DEFAULT:-stable}`, sanitized; app / helpers / entitlements inherit it. (Attach target = right process.)
2. **Reopen handler (Rust/AppKit, `agentmux-cef`)** — on `applicationShouldHandleReopen:` / `kAEReopenApplication`, POST `open_new_window` to self. Fixes "not responding" **and** makes a plain double-click open a window. (Attach action.)
3. **Unix forward (`agentmux-launcher/src/main.rs:585`)** — call `forward_open_new_window` before exiting, mirroring the Windows path, for the `open -n` / CLI route.

(1) makes the OS route to the correct instance; (2) makes the instance actually open a window when reactivated; (3) covers the explicit new-process route.

---

## 6. Out of scope (tracked separately)
- macOS port of `wipe-old-data-dirs.sh` (symptom 3 — disk leakage cleanup).

## 7. Status (verified on-device 2026-06-18)

- **#1 channel-scoped bundle id — ✅ verified.** A `.stable` build launches alongside a running `ai.agentmux.cef` 0.45.0 with **no "not responding"**; LaunchServices registers it as a distinct app, and helpers inherit `…stable.helper.*`.
- **#3 unix `open_new_window` forward — ✅ verified.** `open -n` of a second instance opens a new window in the running instance, and the second launcher forwards then exits.
- **#2 reopen handler — ✅ fixed (follow-up, 2026-06-18).** The first attempt — a raw `NSAppleEventManager` `kAEReopenApplication` handler — was **inert**: CEF re-registers its own AE handler after ours, so it never fired. The working fix installs a dedicated **NSApplication delegate** implementing `applicationShouldHandleReopen:hasVisibleWindows:` (CEF sets no delegate of its own — confirmed by the `reopen-hook: installed dedicated reopen delegate (NSApp had none)` log) and opens a new window. Verified on-device: an `open`/reopen of the running build logs `reopen-hook:fired` and a second window appears (renderer count 2→3). A plain Finder/Dock double-click of the running app now opens a new window; `open -n` and the in-app **"+ Open another window"** remain alternatives.
