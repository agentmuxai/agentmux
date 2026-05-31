# SPEC: Launcher in packaged macOS builds + restore the splash + tear-off crash

**Date:** 2026-05-31
**Repo state:** branch `agenta/cef-148-bump` (CEF 148 patched framework, notarized DMG #1221), base `main` @ v0.40.x
**Author:** AgentO-asaf (driven by Claude)
**Status:** Spec — ready to implement (phased)
**Motivated by:** a SIGABRT crash tearing off a pane from a 2nd window on the packaged macOS DMG, and the absence of the startup splash.
**Builds on:** [`SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30.md`](./SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30.md) (launcher in macOS **dev**), [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md), [`SPEC_MACOS_PACKAGING_2026_05_30.md`](./SPEC_MACOS_PACKAGING_2026_05_30.md), [`SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md`](./SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md).

---

## 0. TL;DR

1. **The owner is right: the launcher is essential**, not just a splash host. The launcher owns a **window/pool/instance reducer + a durable saga coordinator** (`agentmux-launcher/src/reducer/`, `src/saga/`). The host talks to it over IPC (`agentmux-cef/src/launcher_ipc.rs`); when the launcher is absent, **every `report_*` call silently no-ops** and the window-pool / tear-off / instance-numbering / supervision machinery runs degraded and untested.
2. **Packaged macOS bundles only the host** (`scripts/package-macos.sh`, `CFBundleExecutable = agentmux-cef`). No launcher → no single-instance enforcement, no srv supervision (orphan-prone), no saga coordination, no instance numbering, weaker tear-off path. **This is the architectural gap to close.**
3. **The splash MUST be launcher-owned and instant.** The whole point of the tiny launcher is that it is up in milliseconds and can paint a splash *before* the multi-second CEF host load. A host-owned splash is rejected: by the time `agentmux-cef` reaches `main`, the delay the user is staring at has already elapsed. The Win32 splash (`agentmux-launcher/src/splash.rs`, `#![cfg(target_os = "windows")]`) is a layered Win32 window and can't be reused — macOS needs a **new native AppKit splash in the launcher**. This makes "the launcher is the macOS entry point" a **hard prerequisite** for the splash.
4. **The tear-off crash is a separate, host-side bug** that must be root-caused with symbols. It is *plausibly aggravated* by the missing launcher (the pooled tear-off path is launcher-coordinated; without it the host falls back to the less-tested cold `CreateWindowTask` path) but that is **not yet proven**. Treat crash-hardening as its own workstream.

Workstreams: **(A) bundle the launcher as the macOS entry point** — split into **A0** (entry point + instant splash, the priority slice) and **A1/A2** (full single-instance + saga-IPC parity); **(C) root-cause + harden the tear-off crash**. (The former "Workstream B / native splash" is folded into A0, since the splash *requires* the launcher to be the entry point.)

---

## 1. The crash (evidence)

macOS crash report `~/Library/Logs/DiagnosticReports/agentmux-cef-2026-05-31-111746.ips`:

```
proc:        agentmux-cef  (/Volumes/VOLUME/AgentMux.app/Contents/MacOS/agentmux-cef)   ← the DMG build
exception:   EXC_CRASH (SIGABRT)
asi:         libsystem_c.dylib: "abort() called"
crashed thread 0: CrBrowserMain
  abort
  Chromium Embedded Framework  (+0x7aca000 …)   ← all frames in CEF, symbols stripped
```

Repro: open a 2nd window, drag-tear a pane → `abort()` on the **browser-main thread inside CEF**, during window/view creation.

**Symbolication is blocked** because the packaged framework was stripped for the lean DMG (PR #1221). The unstripped from-source framework exists at `~/cef-build/chromium/chromium/src/out/Release_GN_arm64/`, but `symbol_level=1` yields no usable `atos` debug map. **Action (C0): produce a symbolized repro** — see §4.

What we *can* say: a `SIGABRT` on `CrBrowserMain` is a deliberate `abort()` — a Chromium `CHECK`/`DCHECK`, or a Rust `panic!`/`unwrap`/`expect` inside a CEF callback (which aborts the process). The suspect host code is the window-creation path that runs during tear-off (§4).

---

## 2. Rigorous findings — what the launcher owns, and what breaks without it

> Investigated against the **real** repo `/Users/asafebgi/Workspace/agentmux` (an earlier analysis used a stale April-18 checkout at `~/agentmux` and wrongly concluded "the launcher does nothing" — disregard that).

### 2.1 Precise state-ownership map (correcting the mental model)

The owner's phrasing — "the reducer keeps track of panes and state" — spans **two distinct reducers**:

| State | Owner (source of truth) | Notes |
|---|---|---|
| **Pane layout within a window** (splits, the tile tree) | **Frontend** `LayoutModel` (SolidJS reducer) + **srv** (persisted WaveObject) | Not launcher-owned. Survives launcher absence. |
| **Windows, the window pool, instance numbers, HWND↔label links, process lifecycle** | **Launcher** `reducer/` + `saga/` (with a **host-side mirror** via IPC) | **This** is launcher-owned and is what degrades without the launcher. |

So the launcher is essential for the **window/pool/instance** layer, not the **intra-window pane tree**. Both are "panes and state" colloquially; the distinction matters for what the crash and the fix touch.

### 2.2 Launcher reducer + sagas (`agentmux-launcher/src/`)

- **`reducer/{mod,window,pool,connection}.rs`** — owns a read-only **mirror of top-level windows**, the **pool inventory** (unpromoted pre-warmed windows), the **authoritative instance registry** (per-label monotonic numbering), backend-window-id map, and process lifecycle records. Actions: `Register`, `ReportWindow{Opened,Closed}`, `ReportPoolWindow{Added,Removed,Promoted}`, `ReportHwnd*` (WRR drift observability), `ReportHostCounts` (drift check), etc.
- **`saga/{mod,pool_respawn,window_cleanup,recovery}.rs`** + **`saga/log/`** — a Tokio multi-step coordinator with **per-saga deadline timers** and a **durable SQLite log** (`SPEC_LAUNCHER_SAGA_DURABILITY`). Two production sagas:
  - **`pool_respawn_on_promote`** — on tear-off (a pool window is promoted), issue `SpawnPoolWindow` to the host and wait for the refill. Keeps the pre-warm pool topped up *atomically*, bracketed by `SagaStarted`/`SagaCompleted` the renderer buffers on.
  - **`window_cleanup_cascade`** — on window close, issue `ReapPanes` + `DrainPoolIfLast` to the host and await echoes (30 s deadline).
  - **`recovery`** — on launcher startup, re-run unresolved sagas from the durable log.

### 2.3 Host ↔ launcher IPC and the silent-no-op fallback

`agentmux-cef/src/launcher_ipc.rs:78` gates the entire IPC layer on the `AGENTMUX_LAUNCHER_PIPE` env var:

```rust
let pipe_path = match std::env::var("AGENTMUX_LAUNCHER_PIPE") {
    Ok(p) if !p.is_empty() => p,
    _ => { /* "AGENTMUX_LAUNCHER_PIPE unset — running without launcher IPC" */ return None; }
};
```

When unset, `COMMAND_TX` (`OnceLock`, line 37) is never set, so **every `report_window_opened` / `report_pool_window_promoted` / `report_hwnd_opened` becomes a silent no-op**. The host keeps running, but the launcher mirror, the pool sagas, instance numbering, and WRR drift detection all go dark.

### 2.4 The window pool & tear-off: pooled (launcher-coordinated) vs cold (direct)

- **Pooled path** (normal tear-off): `commands/drag.rs::open_window_at_position` → `commands/window_pool.rs::promote_pool_window` → pops a pre-warmed window and fires **6+ launcher reports** (`report_pool_window_removed/promoted`, `report_window_opened`, `report_hwnd_opened`, `compute_and_report_host_counts`). The `pool_respawn` saga then refills.
- **Cold path** (pool empty, or pooling unavailable): falls back to `ui_tasks.rs::CreateWindowTask::execute` (≈ lines 672–766) — direct `browser_view_create` + `window_create_top_level`, no pool, no saga.

**Without the launcher**, the pooled reports no-op and there is **no `pool_respawn` saga to refill the pool**. The pool drains and subsequent tear-offs hit the **cold `CreateWindowTask` path** — which is exactly the suspect crash site (§4) and the least macOS-tested path.

### 2.5 Current state by platform

| | Single-instance | srv lifecycle | Saga/reducer IPC | Splash |
|---|---|---|---|---|
| **Windows (packaged & dev)** | Launcher named-pipe bind | Launcher-spawned sibling, Job Object J0 `KILL_ON_JOB_CLOSE`, crash-budget retry | Wired (named pipe) | Win32 splash |
| **macOS dev** (`task dev`, run_unix, PR #1193) | Chromium process-singleton (host) | **Launcher**-spawned, process-group containment | **NOT wired yet** — Unix socket is Phase 2 of the dev spec; `run_unix` comment: *"wires the Unix-socket transport"* (future). `launcher_ipc` still no-ops. | none |
| **macOS packaged** (the DMG) | Chromium process-singleton (host) | **Host**-spawned (`sidecar.rs`); **orphan-prone** if host crashes | **Absent** (no launcher bundled) | none |

So packaged macOS is the most degraded: no launcher at all. Even dev lacks the saga IPC until Phase 2 lands.

### 2.6 What packaged macOS loses without the launcher (impact table)

| Lost capability | Source | User-visible impact |
|---|---|---|
| **Window instance numbering** | `launcher/reducer/window.rs` | 2nd+ windows have no instance id → InstancePanel labels incomplete |
| **Pool refill saga + bracket** | `saga/pool_respawn.rs` | Pool drains; tear-offs fall to the cold path (crash-suspect); renderer never sees refill brackets |
| **HWND↔label / WRR drift detection** | `window_pool.rs` reports | Torn-off windows can't be matched on close → false "orphan" tracking |
| **Durable saga recovery** | `saga/log/`, `saga/recovery.rs` | A crash mid-tear-off leaves stale pool windows with no recovery breadcrumb |
| **srv supervision + containment** | `launcher main.rs` Job Object / process group | Host crash orphans srv → stale srv can corrupt the next instance's data dir |
| **Crash-budget retry ladder** (`--disable-gpu` rungs) | `launcher main.rs::spawn_host_supervised` | Cascading GPU crashes don't auto-fall-back |
| **Single-instance owned by a supervisor** | `launcher ipc/server.rs` + `data_dir.rs` | Second launch races on the data dir instead of forwarding `open_new_window` |

**Verdict:** the launcher is required for correct, supervised multi-window behavior on macOS. Shipping the host standalone is the root architectural defect behind the fragile tear-off, regardless of whether it is the *direct* SIGABRT cause.

---

## 3. Workstream A — bundle the launcher as the macOS entry point

Goal: packaged macOS runs `agentmux-launcher` as `CFBundleExecutable`. It paints the splash **instantly** (A0), then supervises srv + host exactly like Windows, with single-instance + saga/reducer IPC wired (A1/A2). This is the **packaging + Phase-2/3 completion** that `SPEC_LAUNCHER_MACOS_DEV_INTEGRATION` explicitly deferred ("unblocks a future `task package:macos`").

### A0 — PRIORITY SLICE: launcher as entry point + instant splash

The minimal change that delivers the splash the user wants, without waiting on full IPC parity. The launcher already spawns srv + host on Unix (`run_unix`, landed PR #1193); A0 makes it the *bundled entry point* and gives it an AppKit splash.

**A0.1 — Native AppKit splash (`agentmux-launcher/src/splash_mac.rs`, `#[cfg(target_os = "macos")]`).**
- The **first thing** `main()` does on macOS — before `run_unix`, before srv/host spawn, before any heavy work — is create a minimal `NSApplication`, a **borderless transparent `NSWindow`** centered on the main screen showing the AgentMux brain logo (reuse the BGRA logo already compiled in `build.rs`), and `orderFront`. Target: pixels on screen in **< 100 ms** from app launch.
- **No main-thread/CEF conflict:** the launcher is a *separate process* from the CEF host. The launcher owns a tiny AppKit runloop only for the splash; the host owns its own `NSApplication`/CEF runloop in its child process. (This is exactly how the Win32 splash already works — separate launcher process, host is a child.)
- **No duplicate Dock tile:** the launcher runs as an **accessory app** (activation policy `.accessory` / `LSUIElement`-equivalent) so the splash gets no Dock tile; the host sets `.regular` (existing `set_macos_activation_policy_regular`) and owns the one Dock tile. Validate the Dock tile appears once, from the host.
- **Threading:** AppKit must run on the launcher's main thread. Spawn srv/host supervision (`run_unix`, Tokio) on a background thread (or pump the AppKit runloop alongside Tokio) so the splash stays responsive while the host loads.

**A0.2 — Dismiss protocol.** Generalize the Win32 `AGENTMUX_SPLASH_EVENT` signal to a cross-platform one. Minimal macOS mechanism (no full saga IPC needed yet): the host signals "first frame painted" and the launcher closes the splash + terminates its splash runloop. Options, simplest first:
  1. **stdout/pipe token** — launcher already owns the host child; host prints a `SPLASH-READY` line on first paint, launcher watches the child's stdout. Zero new transport.
  2. Unix-domain datagram to a path passed via env (`AGENTMUX_SPLASH_SOCK`).
  - Add a **safety timeout** (e.g. 8 s) and **dismiss-on-host-window-visible** fallback so a missed signal never leaves the splash stuck.
  - Generalize the host hook in `agentmux-cef/src/client/mod.rs` (today Win32-only `OpenEventW`/`SetEvent`) to emit the chosen macOS signal on first `on_paint`/first window shown.

**A0.3 — Packaging.** Bundle `agentmux-launcher` (see A.2 layout), set `CFBundleExecutable = agentmux-launcher`, sign it inside-out (launcher last), re-notarize + staple. A0 does **not** require the Unix socket / flock work — the launcher can spawn srv+host via the existing `run_unix` path with the splash on top.

**A0 acceptance:**
- [ ] Double-clicking the DMG shows the splash within ~100 ms, before any window.
- [ ] Splash dismisses on host first frame; never stuck (timeout + window-visible fallback verified).
- [ ] Exactly one Dock tile, owned by the host.
- [ ] srv is launcher-spawned (supervision parity gained for free); quit reaps the tree.
- [ ] DMG re-notarized + stapled, `spctl` = Notarized Developer ID.
- [ ] No new OS permission prompts (per the "no notices unless user-initiated" constraint).

### A1/A2 — full single-instance + saga-IPC parity (follow-on)
The dev spec's Phase 1 (launcher supervises srv+host on macOS) has landed (`run_unix`). **Phase 2 (single-instance `flock` + Unix-socket IPC, `AGENTMUX_LAUNCHER_SOCK`)** and **Phase 3 (supervision parity + host parent-death backstop)** complete the reducer/saga coordination — without them the window/pool/instance machinery still no-ops even with the launcher present.

- Add the Unix-socket transport (`ipc/unix_socket.rs`); have the host connect when `AGENTMUX_LAUNCHER_SOCK` is set. **Update `launcher_ipc.rs` to honor `AGENTMUX_LAUNCHER_SOCK` (Unix) in addition to `AGENTMUX_LAUNCHER_PIPE` (Windows)** — today it only checks the pipe var, so the host can never connect on macOS.
- `flock(<data-dir>/launcher.lock)` single-instance + forward `open_new_window` to the running instance.
- `setpgid` + `killpg`-on-exit containment; host parent-death watcher as backstop.

### A.2 Bundle layout
Extend `scripts/package-macos.sh` so the `.app` is launcher-rooted:

```
AgentMux.app/Contents/
  MacOS/
    agentmux-launcher              ← CFBundleExecutable (NEW)
    agentmux-cef                   ← host (no longer the entry point)
    agentmux-srv-<ver>-darwin.arm64
    *.dylib (GL libs), frontend/   ← as today
  Frameworks/Chromium Embedded Framework.framework
  Info.plist                       ← CFBundleExecutable = agentmux-launcher
```

- Build `agentmux-launcher` for darwin (`cargo build -p agentmux-launcher`; today `build:host:darwin` builds only `-p agentmux-cef`).
- The launcher resolves host + srv as **siblings in `Contents/MacOS/`** (not a `runtime/` subdir — packaged layout differs from dev's `dist/cef-dev/runtime/`). Confirm the host's `../Frameworks` framework lookup still resolves with the host at `Contents/MacOS/agentmux-cef` (it does — `Contents/MacOS/../Frameworks`). **This is the layout subtlety; pin it (Open Q1).**
- **Signing:** all five helper apps + host + srv + launcher + dylibs + framework signed inside-out, launcher signed last; entitlements unchanged; re-notarize/staple. The launcher is just another Mach-O in the bundle.
- **Self-spawn guard:** the launcher already refuses to spawn if the resolved host == its own path (`main.rs`). Keep, since both now live in `Contents/MacOS/`.

### A.3 Acceptance (A)
- [ ] Packaged `.app` launches `agentmux-launcher`; window comes up identically.
- [ ] Logs show launcher supervising: srv ESTART adopted by host (`use_launcher_endpoints`), host spawned with `AGENTMUX_BACKEND_*` + `AGENTMUX_LAUNCHER_SOCK`.
- [ ] `launcher_ipc` **connects** (no "running without launcher IPC" line); `report_*` reach the launcher; instance numbering works for the 2nd window.
- [ ] Quit → srv + host + renderers reaped, no orphan srv.
- [ ] Second launch on same data dir → forwards `open_new_window`, exits.
- [ ] Pool refill saga runs on macOS (tear-off keeps the pool topped up); durable log + recovery work.
- [ ] DMG re-notarized + stapled; `spctl` = Notarized Developer ID. Size budget honored (launcher binary is small).
- [ ] Windows unchanged.

---

## 4. Workstream C — root-cause + harden the tear-off crash

Independent of A (the crash must be fixed even with the launcher, since the cold path still exists).

- **C0 — symbolized repro (blocking).** Rebuild/keep an **unstripped framework or a `.dSYM`** (bump `symbol_level=2` for a debug framework, or `dsymutil` the binary) and re-run the tear-off repro to get a real stack. Alternatively run the **prebuilt official cef-dll-sys framework** (which has downloadable symbols) to bisect whether the crash is specific to our patched 148 build or general to the host multi-window path. Until C0, the rest is hypothesis.
- **C1 — harden `CreateWindowTask::execute` (`ui_tasks.rs` ≈710–759).** It selects a client from the first non-pane browser and, if none exists, calls `browser_view_create(None, …)` / `window_create_top_level` anyway. Add an explicit guard: if no live top-level client/browser context is available (all closing / mid-teardown), **bail with a logged error instead of proceeding into CEF** — and verify whether CEF actually tolerates a `None` client here on macOS 26 (it may `CHECK`).
- **C2 — audit `unwrap`/`expect`/`panic!`/`assert!` on the multi-window + tear-off path** (`commands/drag.rs`, `commands/window_pool.rs`, `client/mod.rs` `on_after_created`/`on_before_close`, `ui_tasks.rs`). A panic in any CEF callback aborts on `CrBrowserMain` exactly as observed. Convert hot-path panics to recoverable errors.
- **C3 — macOS multi-window maturity gaps.** Several macOS branches are stubs vs full Windows impls: monitor work-area, tear-off hit-test, focus. Cross-reference `SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md`; fill the gaps the 2nd-window tear-off path hits.
- **C4 — does the launcher fix it?** After A, re-test: with the pool kept warm by the `pool_respawn` saga, does the cold path (and the crash) stop being reached? If the crash persists with the launcher, it is purely host-side (C1/C2/C3). Record the result.

### Acceptance (C)
- [ ] Symbolized stack identifies the exact abort.
- [ ] Open 2nd window + tear-off pane, repeated 20×, no abort (with **and** without the launcher).
- [ ] No `unwrap`/`expect` reachable on the tear-off hot path.

---

## 5. Splash — folded into A0 (launcher-owned)

The splash is **not a separate workstream** — it is the payload of **A0** (§3). The owner's requirement settles the long-open "splash owner" question decisively: it must be the **launcher**, because the launcher is the only process up early enough to paint *before* CEF loads. A host-owned splash is rejected (the host *is* the slow part). See **A0.1/A0.2** for the AppKit design and dismiss protocol. The dev spec §3.6 ("skip splash on Unix") is **superseded** for packaged macOS.

---

## 6. Sequencing & risks

**Recommended order:**
1. **A0** — launcher as macOS entry point + instant AppKit splash (delivers the splash the owner is asking for; gains srv supervision for free). Ship as its own PR + re-notarized DMG.
2. **C0/C1/C2** — symbolize and harden the tear-off crash (independent; the cold `CreateWindowTask` path exists regardless of the launcher).
3. **A1/A2** — single-instance (`flock`) + Unix-socket saga-IPC, so the window/pool/instance reducer actually coordinates on macOS.
4. **C4** — confirm the warm pool (now refilled by the `pool_respawn` saga) keeps tear-off off the crash path.

| Risk | Mitigation |
|---|---|
| Launcher AppKit splash + Tokio supervision on one process | Splash on the launcher main thread; run `run_unix` supervision on a background thread / pump both runloops. Launcher and host are separate processes, so no conflict with CEF's runloop |
| Duplicate Dock tile (launcher + host) | Launcher = `.accessory` (no tile); host = `.regular` (one tile). Verify there's exactly one |
| Splash never dismissed (missed host signal) | Safety timeout (~8 s) + dismiss-on-host-window-visible fallback |
| `../Frameworks` lookup breaks under launcher-rooted bundle | Host stays at `Contents/MacOS/agentmux-cef`; `../Frameworks` already resolves — smoke-test first (Open Q1) |
| Adding the launcher regresses notarization/Gatekeeper | Sign inside-out, launcher last; re-run full `spctl`/`stapler` checks |
| Crash is in our patched CEF 148, not host logic | C0 bisect against the official prebuilt framework |

## 7. Open questions
1. **Framework placement** under the launcher-rooted bundle — confirm `Contents/MacOS/../Frameworks` resolves with the host as a sibling of the launcher (expected yes).
2. **Splash dismiss transport for A0** — host-stdout token (simplest, no new transport) vs a dedicated `AGENTMUX_SPLASH_SOCK` datagram. Start with stdout token; revisit if A1/A2's Unix socket lands first.
3. **AppKit crate** — what the launcher already pulls in for macOS GUI (objc2 / cocoa / raw `objc`); pick the lightest path to an `NSWindow` + image view to keep the launcher tiny and fast.
4. **Is the SIGABRT in our patched framework or generic?** — resolve via C0 before investing in C1/C3.
5. **Linux** — A1/A2's Unix code (flock, Unix socket, process group) is shared; Linux splash is out of scope for now (no instant-splash requirement stated).

## 8. Decision
The launcher is **not optional** on macOS, and the splash **must** live in it. Ship **A0** first (launcher as the packaged entry point with an instant native splash) as the priority — it directly answers "we need it right away." Then harden the crash (C), then complete single-instance + saga-IPC parity (A1/A2) so the window/pool/saga reducer coordinates on macOS. The host-owned-splash fallback is dropped per the owner's instant-splash requirement.
