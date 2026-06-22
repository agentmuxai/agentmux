# SPEC: macOS reopen → open a new window (kill "AgentMux is not responding")

- **Date:** 2026-06-22
- **Status:** Proposed — open questions resolved with evidence (see §11)
- **Author:** AgentO (Masty)
- **Area:** `agentmux-launcher` (macOS), `agentmux-cef` (macOS)
- **Related:** `docs/specs/SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md` (PR #1568); host reopen handler PR #1572; dock-click focus PR #1653

---

## 1. Summary

Double-clicking `AgentMux.app` in Finder (or `open /Applications/AgentMux.app`
without `-n`) **while AgentMux is already running** must open **another window** in
the running instance. Today it instead shows the macOS dialog:

> *You cannot open the application "AgentMux" because it is not responding.*

This spec defines why, and the change that makes a reopen deterministically open a
new window in the correct already-running instance.

This is the deferred follow-up from `SPEC_MACOS_LAUNCH_COHERENCE` ("plain
double-click → new-window reopen handler"). PR #1572 *attempted* it but installed the
handler in the wrong process (see §4); this spec puts it where the OS actually
delivers the event.

---

## 2. Current behavior (symptom)

| Trigger | OS delivers reopen to | Result today |
|---|---|---|
| Finder double-click on a running app | **launcher** (no handler) | "not responding" dialog (timeout) ← reported bug |
| `open AgentMux.app` (no `-n`) | **launcher** (no handler) | same |
| Dock single-click | **host** (#1572 handler) | focuses the front window (no new window) |
| `open -n AgentMux.app` | fresh process launch | new window via socket forward (already works) |
| In-app "+ new window" | in-process command | works |

The OS sends a *reopen* Apple Event to one of the bundle's two processes and waits
for it to be handled. For the Finder/`open` path it targets the **launcher**, which
has no handler, so LaunchServices times out and reports the app as unresponsive.

---

## 3. Process & single-instance model (facts)

macOS packaged bundle (`scripts/package-macos.sh`):

- **`CFBundleExecutable = agentmux-launcher`** (`package-macos.sh:228`). The
  **launcher is the process LaunchServices launches** for the bundle.
- The launcher spawns **`agentmux-cef`** (the host) as a child and supervises it
  (`run_unix`, `agentmux-launcher/src/main.rs:764`).
- **Activation policy is split:** the launcher runs as an *accessory*
  (`setActivationPolicy: 1`, no Dock tile — `splash_mac.rs build_window`); the
  **host** sets `.regular` and **owns the Dock tile**.

**LaunchServices topology (observed live via `lsappinfo list`):** one bundle id,
**two** registered application ASNs —

```
launcher  pid=1144  executable=agentmux-launcher  type="UIElement"   parentASN="Dock"
host      pid=1259  executable=agentmux-cef        type="Foreground" (in front)  parentASN=launcher
```

The launcher is the LS-launched main executable (UIElement, rooted at the Dock); the
host is a child ASN that is the actual foreground app and owns the Dock tile.

Threading inside the launcher process on macOS:

- **Main thread:** creates `NSApplication` (accessory) for the splash, then parks in
  `Splash::run_until_dismissed()` pumping `CFRunLoopRunInMode(default, …)` forever
  (`splash_mac.rs`). AppKit delegate callbacks (incl. reopen) would be delivered here.
- **Supervisor thread:** `run_unix` resolves paths, **binds the single-instance unix
  socket**, and spawns srv + host.

Single-instance identity (the socket the running instance owns):

```
version  = AGENTMUX_IPC_VERSION_OVERRIDE | CARGO_PKG_VERSION         (main.rs:815)
data_dir = data_dir::resolve_paths(exe_dir, version).data_dir        (main.rs:775)
dir_hash = hash::data_dir_hash16(data_dir, version)                  (main.rs:816)
socket   = ipc::pipe_name(dir_hash)                                  (main.rs:817)
```

`resolve_paths` derives `data_dir` from the **baked** channel
(`AGENTMUX_BUILD_CHANNEL_DEFAULT`) and **deliberately ignores a leaked ambient
`AGENTMUX_CHANNEL`** for nested/dev launches (`data_dir.rs:77-99`); an explicit
standalone override is still honored. So `(channel, version)` → `data_dir` →
`dir_hash` → `socket` is deterministic per build.

The forward primitive already exists, used when a *second* launcher loses the
socket-bind race:

- `forward_open_new_window(data_dir, dir_hash)` — connect to `socket`, send
  `{"cmd":"open_new_window"}` (`main.rs:2013`).
- `forward_open_new_window_or_log(data_dir, dir_hash)` — same, logging on failure
  (`main.rs:665, 698`).

A reopen handler already exists **in the host** (`agentmux-cef`):
`install_reopen_handler` / `should_handle_reopen`
(`agentmux-cef/src/macos_menu.rs:148,180`, installed at `lib.rs:922`). It opens a new
window when `hasVisibleWindows == NO`, else focuses the front window — this is the
**Dock-click** path.

---

## 4. Root cause

1. **The Finder/`open` reopen is delivered to the launcher, which has no handler.**
   The launcher is the CFBundleExecutable LaunchServices launched, so `open`/Finder
   resolves the running bundle to the **launcher's** ASN and sends it the `'rapp'`
   event. The launcher never installs `applicationShouldHandleReopen:`, so the event
   is unhandled and the OS times out → "not responding."
   *Proof:* the running 0.47.3 **host** binary already contains the #1572 handler
   (`reopen-hook` symbols present), yet a Finder double-click still produces the
   dialog. If the event reached the host it would be handled (focus/open). It is not
   → the target is the launcher. The launcher binary contains **no** `reopen-hook`.
2. **PR #1572 put the handler in the wrong process.** Its intent — "plain
   double-click of a running app opens a new window" — is correct, but the host only
   receives the **Dock-click** reopen, not the Finder/`open` one. So #1572 currently
   only affects Dock clicks (and there it *focuses*, per #1653), never the
   double-click it was named for.

---

## 5. Goals / non-goals

**Goals**
- Finder double-click / `open` (no `-n`) on a running instance opens a **new
  window** in that instance. No "not responding" dialog, ever.
- The reopen is routed to the **correct** running instance — same `(channel,
  version)` — by reusing the exact `dir_hash`/socket the instance bound (the user's
  "know its channel/version to bind correctly").

**Non-goals**
- Cross-version/channel isolation (done in `SPEC_MACOS_LAUNCH_COHERENCE`). Two stable
  releases still share `ai.agentmux.cef.stable` by design; reopen only ever targets
  the same bundle id.
- Windows/Linux behavior change (Windows already forwards via the named pipe; §10).

---

## 6. Design

Resolved routing (§11 OQ-1) means **two trigger paths, two processes** — both must
be handled, and both must end in "open a new window in the right instance":

| Path | Process | Handler | Action |
|---|---|---|---|
| Finder double-click / `open` (no `-n`) | launcher | **NEW (this spec)** | forward `open_new_window` over the bound socket |
| Dock-tile click | host | existing (#1572) | open a new window in-process |

### 6.1 Launcher reopen handler (the missing piece)

Install `applicationShouldHandleReopen:hasVisibleWindows:` on the **launcher's**
`NSApplication` (the splash's NSApp), on the **main thread**, after the NSApp is
created in `Splash::show()`. Reuse the Chromium-proof delegate technique already in
`agentmux-cef/src/macos_menu.rs::install_reopen_handler` (add/override the delegate
method via `class_addMethod` / `method_setImplementation`; install a dedicated
`NSObject` delegate if NSApp has none).

The launcher cannot create a CEF window itself, so it **forwards** to the host:

```
fn on_reopen() -> handled(NO) {
    match REOPEN_TARGET.get() {                       // (data_dir, dir_hash), see §6.3
        Some((data_dir, dir_hash)) => forward_open_new_window_or_log(data_dir, dir_hash),
        None => { /* host still starting; see §6.4 — no-op */ }
    }
    return NO;  // handled; AppKit must not also run its default reopen
}
```

Always forward — do **not** branch on `hasVisibleWindows` (OQ-2: double-click always
opens a new window).

### 6.2 Host reopen handler (Dock-click path) — unchanged behavior

Keep `should_handle_reopen` as-is for the Dock path: focus the front window when one
is visible, open a new window when none are. This is native macOS Dock behavior and
is what a Dock click should do. (Per OQ-2 the *double-click* opens a new window; the
Dock click keeps the native focus-on-visible gesture. If we later want Dock clicks to
also always open a window, drop the `hasVisibleWindows` branch — a one-line change —
but that diverges from the platform norm and is explicitly **not** done here.)

### 6.3 Channel/version-correct binding (the user's explicit requirement)

The launcher handler MUST target the **same socket the running instance bound** — not
a freshly recomputed one. `run_unix` already computes `(data_dir, dir_hash,
socket_path)` on the supervisor thread (`main.rs:775,816,817`). Publish those to the
main thread the moment the socket is bound:

```
// set immediately after bind_socket_with_recovery wins the socket (main.rs ~856)
static REOPEN_TARGET: OnceLock<(PathBuf, String)> = OnceLock::new();   // (data_dir, dir_hash)
let _ = REOPEN_TARGET.set((paths.data_dir.clone(), dir_hash.clone()));
```

The handler reads `REOPEN_TARGET` and forwards. **Do not** re-run
`resolve_paths`/`data_dir_hash16` inside the handler — recomputation could diverge
from the bind (a leaked `AGENTMUX_CHANNEL`, or an `AGENTMUX_IPC_VERSION_OVERRIDE`
present at bind time but not handler time). The bound value is authoritative; reuse
it verbatim so the forward always lands in the right `(channel, version)` instance.

### 6.4 Reopen during host startup (socket not yet bound)

A reopen can fire before `run_unix` binds the socket. If `REOPEN_TARGET` is unset,
the handler **no-ops and returns NO** — the first window is already coming up; a
second window for a not-yet-ready instance is undesirable. (Optional: a bounded
retry, e.g. 3×250 ms, never blocking the main thread.)

### 6.5 Run-loop delivery — REQUIRED (verified)

A bare `CFRunLoopRunInMode(kCFRunLoopDefaultMode, …)` park does **NOT** deliver the
reopen Apple Event to the delegate. Confirmed empirically: with the handler
installed but the launcher parking on a raw CFRunLoop, `open -b <bundleid>`
returned **`-1712` (`errAETimeout`)** and the handler never fired — i.e. exactly
the "not responding" timeout. In Cocoa the `'rapp'` event is drained through
NSApplication's event pump (`nextEventMatchingMask:`/`[NSApp run]`, which pulls the
AE Mach port via HIToolbox), not a standalone CFRunLoop source.

**Fix (implemented):** both launcher park loops (`run_until_dismissed` — the splash
animation loop and the post-dismiss park) pump `NSApp` instead:
`pump_app_events(seconds)` calls
`[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:(now+seconds)
inMode:kCFRunLoopDefaultMode dequeue:YES]` (sending any returned UI event). The AE
is dispatched to the delegate as a side effect of the run-loop service inside that
call. After this change the same `open -b` returns 0, logs `reopen-hook:fired
proc=launcher`, forwards `open_new_window`, and the host logs a `CreateWindow`.

---

## 7. Implementation sketch

| File | Change |
|---|---|
| `agentmux-launcher/src/splash_mac.rs` | Add `install_reopen_handler()` (delegate add/swizzle, mirrors `macos_menu.rs`); call from `Splash::show()` after the NSApp exists. Body reads `REOPEN_TARGET` and forwards. **Add `pump_app_events()` and use it in both `run_until_dismissed` loops instead of `CFRunLoopRunInMode` (§6.5) — without this the handler never fires.** |
| `agentmux-launcher/src/main.rs` | Define `REOPEN_TARGET: OnceLock<(PathBuf,String)>`; `set` it right after `bind_socket_with_recovery` wins the socket (~`:856`). Expose `forward_open_new_window_or_log` to the splash module (or pass a closure). Log `reopen-hook:fired proc=launcher`. |
| `agentmux-cef/src/macos_menu.rs` | No behavior change. Add `proc=host` to the existing `reopen-hook:fired` log to disambiguate the two paths in `muxlog`. |

No new dependencies; all FFI patterns already exist in the codebase.

---

## 8. Edge cases

- **Rapid repeated double-clicks:** each forward opens one window. Acceptable;
  optionally debounce (ignore reopens within ~300 ms) if testers find it surprising.
- **Host crashed but launcher alive:** the socket connect in `forward_open_new_window`
  fails → logged, no window, no dialog. The supervisor's crash budget handles host
  restart independently.
- **Reopen mid-startup:** §6.4.
- **Different channel/version build double-clicked:** it is a *different* bundle id
  (post-#1568) → the OS launches it fresh; not a reopen of this instance. Unchanged.
- **`open -n`:** still spawns a real second launcher that loses the socket race and
  forwards — unchanged.

---

## 9. Verification plan

1. **Routing confirmation (already evidenced, re-confirm post-change):** build, install
   to `/Applications`, launch. Tail `muxlog launcher` + `muxlog host`. A Finder
   double-click and an `open` (no `-n`) → exactly one `reopen-hook:fired
   proc=launcher`; a Dock click → `proc=host`.
2. **New window opens, no dialog:** with a visible window present, Finder double-click
   → a second AgentMux window appears; **no** "not responding" dialog.
3. **Correct instance:** with two *different* channel builds installed and both
   running, double-click each → each opens a window in *its own* instance (forward
   targets the matching `dir_hash`). Confirm via per-instance launcher log.
4. **Dock click unchanged:** Dock click with a visible window focuses (no new window);
   with all windows closed, opens one.
5. **Startup race:** double-click during the splash/first-paint window → no crash, no
   dialog, no spurious second window.
6. **Regression:** `open -n` and in-app "+ window" still open windows.

(No offscreen shortcut as for the splash footer — reopen is a LaunchServices
interaction and must be exercised by a real install + click.)

---

## 10. Cross-platform parity

- **Windows:** already forwards `open_new_window` over the named pipe on a second
  launch (`run_windows` / `forward_open_new_window`, `main.rs:1366`). No reopen
  concept; relaunch is the trigger. Unchanged.
- **Linux:** the unix-socket forward (`bind_socket_with_recovery`) handles a second
  launch; a `.desktop` relaunch behaves like `open -n` (fresh process → forward), so
  it already opens a new window. No accessory/host split, no `'rapp'` event. Confirm
  during verification that re-activating the `.desktop` entry forwards rather than
  no-ops.

---

## 11. Resolved questions & follow-ups

- **OQ-1 — Which process receives the reopen? RESOLVED.** Finder double-click and
  `open` (no `-n`) → the **launcher** (the LS-launched `CFBundleExecutable`, a
  UIElement). Dock-tile click → the **host** (the Foreground app owning the tile).
  *Evidence:* `lsappinfo` shows two ASNs for one bundle (launcher = UIElement rooted
  at Dock; host = Foreground "in front"); and the running 0.47.3 host already carries
  the #1572 handler yet a double-click still times out — so the double-click never
  reaches the host. ⇒ **both** handlers are required; neither is dead (this removes
  the earlier "delete the dead handler" follow-up).
- **OQ-2 — New window vs focus? RESOLVED.** The double-click/`open` (launcher) path
  **always opens a new window** — the user's request and #1572's original intent. The
  Dock-click (host) path keeps **native macOS behavior** (focus when a window is
  visible). Documented alternative: make Dock clicks also always open a window by
  dropping the host handler's `hasVisibleWindows` branch — intentionally **not** done,
  as it diverges from the platform norm.
- **FU-1 — macOS stale-data-dir cleanup.** Unrelated but adjacent:
  `scripts/wipe-old-data-dirs.sh` is still Windows-only; stale macOS data dirs
  accumulate (tracked in `SPEC_MACOS_LAUNCH_COHERENCE` follow-ups).
