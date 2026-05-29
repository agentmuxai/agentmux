# macOS Tear-off Stability — implementation spec

**Date:** 2026-05-29
**Repo state:** `main` @ `53d781af` (v0.40.0)
**Author:** AgentO-asaf
**Status:** Spec ready to implement
**Closes:** [#1138](https://github.com/agentmuxai/agentmux/issues/1138)
**Source report:** [`docs/analysis/REPORT_MACOS_TEAROFF_DRAG_CRASH_2026_05_29.md`](../analysis/REPORT_MACOS_TEAROFF_DRAG_CRASH_2026_05_29.md)
**Prior art:** [PR #403](https://github.com/agentmuxai/agentmux/pull/403) — `fix(macos): patch NSApplication for macOS 26 Tahoe drag crash` (open since 2026-04-15, mixed scope, can't merge as-is)
**Related (not in scope):** [`SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md`](./SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md) §3.2 (C1 work block — real macOS tear-off implementation, deferred)

---

## Problem

`agentmux-cef` crashes on macOS 26 Tahoe whenever a user initiates any drag gesture that exercises Cocoa's `NSDraggingSession` machinery — pane drag, tab tear-off, sometimes just window-titlebar drag. Six identical crash diagnostics on the dev machine in two days; 100% reproducible. The host process exits in <200 ms with `EXC_BREAKPOINT / SIGTRAP` after `objc_terminate()`. The Rust panic machinery never runs.

Root cause (confirmed via PR #403's investigation): **CEF 146 still calls private `NSApplication` selectors that Apple removed in macOS 26.** `isHandlingSendEvent`, `isSendingEvent`, `setEffectiveAppearance:`, and similar are dispatched during `NSDraggingSession` setup. The receiver (`NSApplication`'s singleton) has no implementation, so the ObjC runtime walks `___forwarding___`, finds nothing, and `objc_exception_throw`s. The default AppKit `NSApplicationUncaughtExceptionHandler` catches it and calls `_objc_terminate()` — the host dies.

Compounding UX issue: the dead host leaves a stale `~/.../cef-cache/SingletonLock` symlink pointing at the now-dead PID. The next `task dev` invocation reads that symlink, treats it as an "existing instance", hands off CLI args via process_singleton, and exits with code 24. The user sees "AgentMux failed to start" with no obvious recovery — they have to find and delete the lock file by hand.

## TL;DR

Two surgical fixes that together make macOS dev usable on Tahoe today, without doing the long-running C1 work:

1. **Inject `+resolveInstanceMethod:` into `NSApplication`'s metaclass** at host startup, before `cef::initialize`. Unknown selectors get typed stubs (BOOL→NO for `isHandlingSendEvent` / `isSendingEvent`, void→() for everything else). The ObjC runtime never enters `___forwarding___`, the throw never happens, drag completes. macOS-only (`#[cfg(target_os = "macos")]`), ~100 lines of `unsafe extern "C"` FFI, ported verbatim from PR #403 with attribution.
2. **Prune stale `SingletonLock` symlinks before launching the host** in `task dev:serve`. ~10 lines of bash that reads the lock target, splits off the PID, and `kill -0`s it; if dead, `rm` the lock + cookie + socket + ipc-port files. Five-second fix to a one-week-long developer paper cut.

Neither implements real macOS tear-off (that's still C1). They make the existing no-op tear-off path *not crash* and recover gracefully when it does.

---

## Why this works

### The metaclass-method injection

CEF on macOS 26 calls APIs that Apple removed. The standard Cocoa response is `NSInvalidArgumentException — unrecognized selector sent to instance 0x118001a3f00`. Three places to intervene:

1. **Override the `NSApplicationUncaughtExceptionHandler`.** Caught after the throw, no useful recovery — the exception is already in flight, autorelease pool is partial, drag session state is undefined. Best for *logging*, not for *preventing*.
2. **Swizzle `-[NSObject doesNotRecognizeSelector:]`.** Called from inside `___forwarding___`. Returning normally without throwing corrupts forwarding state — secondary crash inside `___forwarding___` itself. **Don't do this.** (PR #403's commit history shows they tried; this exact failure mode is documented in its doc-comment.)
3. **Inject `+[NSApplication resolveInstanceMethod:]`.** Called *before* `___forwarding___`. Add a typed stub method to the class on the fly, return `YES`, runtime retries the original message send against the freshly-added method, message lands. This is the documented Apple-blessed path for dynamic message resolution. ← **what we do.**

### Why return-type matters for the stub

`isHandlingSendEvent` returns `BOOL`. On ARM64 the return value lives in `x0`. If we install a `void` stub for it, `x0` carries the leftover value (typically `self`, which is non-nil → truthy). CEF reads that and concludes "the app is already inside a `sendEvent:` call, skip routing" — breaking window drag silently *even after we stop the crash*. PR #403's `bool_no_stub` returns `0` in `x0`; this is the right ABI for `BOOL` selectors.

So the stub-injection function maintains a small allowlist of selectors that must return `BOOL` (currently `isHandlingSendEvent`, `isSendingEvent`). Everything else gets the void stub.

### Why the metaclass, not the class

`+resolveInstanceMethod:` is a *class method*. Class methods live in the metaclass. `object_getClass(NSApplication)` returns the metaclass; that's where `class_addMethod` has to install the resolver. PR #403 has the dance right.

### Why this is forward-compatible

Apple may un-deprecate these selectors, restore them, or change CEF's dispatch path. Our resolver only kicks in for *unknown* selectors — selectors `NSApplication` does implement go through the normal fast path and never reach `+resolveInstanceMethod:`. When the underlying CEF bug is fixed upstream (and we bump to a CEF version that no longer calls the removed selectors), this code is dead but harmless and can be deleted in a one-line follow-up.

### Why we don't need an `NSExceptionHandler` first

The original plan (per the analysis report §7) was to install `NSExceptionHandler` to discover the missing selector. PR #403 already did that discovery — the selectors are known (`isHandlingSendEvent`, `isSendingEvent`, plus an unbounded set of other void-returning selectors caught by the generic stub). We can skip the diagnostic step and go straight to the fix, copying PR #403's code with attribution.

If a *new* missing selector shows up later (different macOS minor release, different CEF version), the `resolve_instance_method_impl` already logs every selector it stubs (`tracing::warn!(selector = %name, "...")`), so the catchall stays observable.

### SingletonLock pruning rationale

Chromium's process_singleton on POSIX is documented to use a `SingletonLock` symlink whose target is `<hostname>-<pid>`. A live instance owns it; on clean exit, it's deleted. On crash, it's stranded. New launches read the target, `kill(pid, 0)`-check whether the PID is alive; if alive, hand off and exit; if **dead**, prune and proceed.

Chromium itself does the live/dead check — but only after it's bound to the lock path; CEF's `process_singleton` *does* do this. Empirically, on macOS the current code path exits with code 24 (CEF_RESULT_CODE_NORMAL_EXIT_PROCESS_NOTIFIED) *without* pruning, so something between Chromium's behavior and CEF 146's macOS-specific build is short-circuiting it. The cheapest workaround is to do the check *outside* the host in shell — `task dev:serve` already has a multi-line bash block that auto-selects a Vite port; adding the SingletonLock check is the same shape.

---

## Detailed design

### Item 1 — `agentmux-cef/src/main.rs`: `patch_nsapp_unrecognized_selector()`

Port verbatim from PR #403 with attribution comment. Structure:

```rust
/// macOS 26 Tahoe compat — see SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md and PR #403.
#[cfg(target_os = "macos")]
unsafe fn patch_nsapp_unrecognized_selector() {
    use std::ffi::{c_char, c_void};

    type Id    = *mut c_void;
    type Sel   = *const c_void;
    type Class = *mut c_void;

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn object_getClass(obj: Id) -> Class;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn sel_getName(sel: Sel) -> *const c_char;
        fn class_addMethod(cls: Class, sel: Sel, imp: usize, types: *const c_char) -> u8;
    }

    unsafe extern "C" fn void_stub(_self: Id, _cmd: Sel) {}
    unsafe extern "C" fn bool_no_stub(_self: Id, _cmd: Sel) -> u8 { 0 }

    unsafe extern "C" fn resolve_instance_method_impl(cls: Class, _cmd: Sel, sel: Sel) -> u8 {
        let name = std::ffi::CStr::from_ptr(sel_getName(sel)).to_string_lossy().into_owned();
        const BOOL_NO_SELECTORS: &[&str] = &["isHandlingSendEvent", "isSendingEvent"];
        if BOOL_NO_SELECTORS.contains(&name.as_str()) {
            tracing::warn!(selector = %name, "macOS 26 compat: adding BOOL(NO) stub");
            class_addMethod(cls, sel, bool_no_stub as usize, b"c@:\0".as_ptr() as _);
        } else {
            tracing::warn!(selector = %name, "macOS 26 compat: adding void stub");
            class_addMethod(cls, sel, void_stub as usize, b"v@:\0".as_ptr() as _);
        }
        1 // YES
    }

    let cls = objc_getClass(b"NSApplication\0".as_ptr() as _);
    if cls.is_null() { tracing::warn!("macOS 26 compat: NSApplication class not found"); return; }
    let metacls = object_getClass(cls as Id);
    if metacls.is_null() { tracing::warn!("macOS 26 compat: NSApplication metaclass not found"); return; }
    let sel = sel_registerName(b"resolveInstanceMethod:\0".as_ptr() as _);
    let added = class_addMethod(metacls, sel, resolve_instance_method_impl as usize,
        b"c@::\0".as_ptr() as _);
    if added != 0 {
        tracing::info!("macOS 26 compat: injected resolveInstanceMethod: into NSApplication metaclass");
    } else {
        tracing::warn!("macOS 26 compat: class_addMethod failed (already exists?)");
    }
}
```

Call site (early in `main()`, *before* `cef::initialize`):

```rust
#[cfg(target_os = "macos")]
unsafe { patch_nsapp_unrecognized_selector() };
```

The right insertion point on current `main` (post-refactor) is after logging setup and before the `cef_app::AgentMuxApp::new` construction — the function only depends on the ObjC runtime, so it's safe at any point before CEF init.

No new crate dependencies. Pure `extern "C"` against `libobjc` (loaded automatically on macOS). PR #403 used this approach to avoid pulling in `objc2` or `cocoa` crates for ~50 lines of FFI; we follow suit.

### Item 2 — `Taskfile.yml::dev:serve`: SingletonLock auto-prune

In `dev:serve`, immediately before the host launch block (after Vite is up, before `./agentmux-cef --url=...`), add a small bash check:

```bash
# Auto-prune stale Chromium SingletonLock if its target PID is dead.
# Without this, a crashed host leaves a symlink pointing at a non-existent
# PID, and the next `task dev` hands off via process_singleton and exits
# with code 24 — looking like "AgentMux failed to start" to the user.
# See SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md §2.
if [ "{{OS}}" != "windows" ]; then
  # Resolve the CEF cache dir — same derivation rule as the host
  # (instance_data_dir / cef-cache).  Locate it via the current
  # agentmux-cef config's instance hash; fall back to a glob.
  for lock in "$HOME/.agentmux/dev/"*/*/cef-cache/SingletonLock; do
    [ -L "$lock" ] || continue
    target=$(readlink "$lock") || continue
    # target is "<hostname>-<pid>"; split.
    pid="${target##*-}"
    [ -n "$pid" ] || continue
    if ! kill -0 "$pid" 2>/dev/null; then
      cache_dir=$(dirname "$lock")
      echo "[singleton] Pruning stale lock $lock (pid $pid is gone)"
      rm -f "$cache_dir/SingletonLock" "$cache_dir/SingletonCookie" \
            "$cache_dir/SingletonSocket" "$cache_dir/ipc-port"
    fi
  done
fi
```

Risk: in the (very narrow) window where a parallel `task dev` from a *different* AgentMux dev clone owns the lock for a different branch, our prune touches it. Mitigation: the glob is scoped to `$HOME/.agentmux/dev/`, the lock check is `-L` (only symlinks, not regular files), the `kill -0` only flags actually-dead PIDs, and the rm only fires when dead. A live, healthy second instance is safe.

### Out of scope

- **Real macOS tear-off (C1 work block).** `agentmux-cef/src/floating_pane_macos.rs` with `NSPanel` + `addChildWindow:ordered:` + native `NSDraggingDestination` view. Estimated ~400 lines per the cross-platform tearoff spec §3.2. Tracked there; not duplicated here.
- **`NSExceptionHandler` install.** Not needed once we have item 1; the BOOL/void allowlist captures the known crashing selectors and the resolver logs new ones if Apple removes more in future macOS releases.
- **CEF upgrade past the Apple-private-selector breakage.** When CEF maintainers fix this upstream and we bump, this code becomes dead — delete it. Until then, this is a load-bearing macOS-only `unsafe` block.
- **Reverting `-webkit-app-region: drag` on macOS.** With item 1 in place, the drag path that the CSS opens up should no longer crash. The CSS stays.

---

## Acceptance criteria

A fresh clone on macOS 26 Tahoe:

```bash
git clone git@github.com:agentmuxai/agentmux.git
cd agentmux
task dev
```

1. Host starts; `muxlog host '"macOS 26 compat: injected resolveInstanceMethod:"'` shows the resolver was installed.
2. Drag a pane header → pane moves, no crash. `muxlog host '"macOS 26 compat: adding"'` shows each missing selector being stubbed once.
3. Drag the window by its title bar → window moves, no crash. `isHandlingSendEvent` and `isSendingEvent` get `BOOL(NO)` stubs (visible in muxlog).
4. Kill the host externally (`kill -9 <pid>`); rerun `task dev`. The launch surfaces `[singleton] Pruning stale lock …` and proceeds normally to a fresh window instead of exiting with code 24.
5. Windows + Linux: zero behavioral change (item 1 is `#[cfg(target_os = "macos")]`; item 2 is `if [ "{{OS}}" != "windows" ]`, and Linux has the same singleton mechanic on POSIX).

---

## PR rollout

Three independent PRs, in dependency order. The first two are already in flight or queued; this spec adds the next two.

| PR | Branch | Base | Contents | Status |
|---|---|---|---|---|
| **PR #1131** | `agenta/macos-host-compile-fix-cef146` | `main` | Compile fix — per-platform `on_pre_key_event` + `patched-libcef` feature gate around `begin_window_drag` | Open |
| **PR (new)** | `agenta/macos-cef-framework-bundling` | `main` | Runtime fix — `bundle:darwin` framework + GL libs, `framework_dir_path`/`resources_dir_path`, keychain switches, traffic-light port, `-webkit-app-region: drag` | Branch pushed, no PR yet — opening as part of this rollout |
| **PR (new)** | `agenta/macos-nsapp-tahoe-compat` | `main` | This spec, item 1 — `patch_nsapp_unrecognized_selector()` | New |
| **PR (new)** | `agenta/macos-singleton-lock-guard` | `main` | This spec, item 2 — SingletonLock auto-prune in `dev:serve` | New |

Independent so each can be reviewed, approved, and merged separately. None of them require the others to land first — the compile fix is necessary for any macOS build to exist, but the NSApp patch is a `#[cfg(target_os = "macos")]` Rust addition that compiles regardless and goes through `cargo check` for all platforms via the existing CI machinery.

Closes [#1138](https://github.com/agentmuxai/agentmux/issues/1138) when item 1 lands. Closes the SingletonLock UX issue (no separate filing yet) when item 2 lands.

---

## Action items

- [x] Author this spec.
- [ ] Open PR for `agenta/macos-cef-framework-bundling` (currently has commits, no PR yet) — runtime fix + traffic lights + drag CSS.
- [ ] Branch + open PR for `agenta/macos-nsapp-tahoe-compat` — port `patch_nsapp_unrecognized_selector` from PR #403 with attribution.
- [ ] Branch + open PR for `agenta/macos-singleton-lock-guard` — auto-prune snippet in `dev:serve`.
- [ ] On approval, merge each.

---

## Acknowledgements

The metaclass-injection approach, the BOOL-vs-void return-type allowlist, the rationale comments, and the diagnosis that this is a macOS 26 Tahoe regression in CEF 146's call to private `NSApplication` selectors — all from **PR #403** (a5af, 2026-04-15). This spec ports the load-bearing piece into a current-main-compatible branch and pairs it with the SingletonLock UX fix.
