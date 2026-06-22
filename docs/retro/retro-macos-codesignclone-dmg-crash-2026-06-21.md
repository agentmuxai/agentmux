# Retro: packaged macOS app crash-loops at startup when launched from a DMG

**Date:** 2026-06-21
**Area:** macOS packaging / CEF host startup
**Severity:** launch-blocking (app never opens a window)
**Fix:** disable Chromium's `MacAppCodeSignClone` feature via `--disable-features`
(`agentmux-cef/src/app.rs`)

## Symptom

After a machine restart, double-clicking the AgentMux app icon produced "funny
behavior" — no window ever appeared. The app looked dead.

Live-state triage (`~/.agentmux/logs/`):

- No `agentmux-launcher` / `agentmux-cef` / `agentmuxsrv` process alive.
- `current-host-v0.47.3.path` pointer **missing** — newest host pointer was a
  stale `v0.42.0` from Jun 5. The host never got far enough into startup to
  register its own log pointer.
- `agentmux-launcher.log` tail showed the launcher coming up as `v0.47.3`,
  spawning the CEF host, and the host dying immediately — four times:

  ```
  spawned CEF host pid=1072 → killed by signal 5 (crash)
    restart 1/3 → crash
    restart 2/3 (degraded: --disable-gpu) → crash   ← rules out GPU
    restart 3/3 (degraded: --disable-gpu) → crash
  restart budget exhausted (3 in 60s) — giving up
  terminating children (SIGTERM → grace → SIGKILL)
  launcher exiting with code 1
  ```

The launcher's crash-budget supervision worked exactly as designed; the host was
the problem.

## Root cause

macOS crash reports (`~/Library/Logs/DiagnosticReports/agentmux-cef-*.ips`, four
at 15:23:12 matching the four crash-loop attempts) pinned it:

```
exception: EXC_BREAKPOINT (SIGTRAP, signal 5)
faulting thread top frame: Chromium Embedded Framework  cef_initialize
  → agentmux-cef main → dyld start
procPath: /Volumes/VOLUME/AgentMux.app/Contents/MacOS/agentmux-cef
os: macOS 26.5.1
```

Two things:

1. The crash is a self-inflicted `CHECK`/`DCHECK` trap (`EXC_BREAKPOINT`, not a
   kernel codesign kill) **inside `cef_initialize`** — stage 0 of the host, long
   before any window is created. That's why no host log pointer and no window.
2. `procPath` is on **`/Volumes/VOLUME`** — the app was being run straight from
   the **mounted DMG**, not from `/Applications`. (Double-clicked the icon inside
   the "drag me to Applications" disk image instead of dragging it out.)

The mechanism is Chromium's **`MacAppCodeSignClone`** feature ("code-sign safe
updates"): on launch, before init completes, Chromium `clonefile()`s the entire
`.app` bundle into a temp dir on the boot volume and runs from that clone, so an
in-place auto-update can replace the on-disk bundle without invalidating the
running process's code signature (lazily-paged code pages would otherwise fail
cdhash validation mid-session and the OS would kill the process).

`clonefile()` is **single-volume** — it cannot span devices. A mounted DMG is its
own volume, separate from the boot-volume temp dir, so the clone fails `EXDEV`,
Chromium `CHECK`-aborts, and the host traps in `cef_initialize`. `--disable-gpu`
on the degraded relaunches didn't help, confirming it's core init, not graphics.

"It used to work" because older AgentMux builds bundled a pre-clone CEF; the
behavior only became fatal after the CEF version bump.

## Fix

`MacAppCodeSignClone` exists to protect against *Chromium's own bundle being
swapped underneath a running process by a background updater*. AgentMux:

- sets `--disable-component-update` (Chromium's updater is off), and
- ships its own launcher/updater that updates on relaunch, never live-swapping
  the running bundle.

So the feature guards against a scenario AgentMux structurally never hits — it is
pure liability (the DMG crash, plus a >1 GB temp clone per run). We disable it by
adding `MacAppCodeSignClone` to the existing `--disable-features` list in
`agentmux-cef/src/app.rs` (`on_before_command_line_processing`). One switch, one
feature name appended to a comma-joined value (a second `--disable-features`
would clobber the first).

## Caveat — verify from a DMG

There is a known trap one block down in the same function: the MachPort peer
policy is read **before** `FeatureList` init, so the runtime flag for it can't
apply in time and a *source patch* was required instead (see
`docs/cef-patches/agentmux_disable_mach_rendezvous_validation.patch`).

`MacAppCodeSignClone` is a genuine `base::Feature` (evaluated *after* FeatureList
init, like the other working entries), so the `--disable-features` switch *should*
land in time. But the clone runs early, so this **must be verified by building and
launching from a DMG**. If it proves too-late like MachPort, the fallback is a
`code_sign_clone_manager` source patch in the from-source CEF, same pattern as the
MachPort patch.

## Lessons

- A `SIGTRAP` / `EXC_BREAKPOINT` in `cef_initialize` from a `/Volumes/...` path is
  the signature of this class of bug — check the crash report's `procPath` first.
- The DMG ships an `/Applications` drag-target symlink for a reason; running
  in-place from the DMG was never supported and is now actively fatal.
- **Stale guidance corrected:** earlier internal advice to "run from `/Volumes`
  or `/Applications`" (to avoid TCC folder prompts) is wrong for current CEF — any
  non-boot volume breaks the code-sign clone. `/Applications` is the only location
  that satisfies both (same device as temp dir *and* no TCC prompts).

## Immediate operator workaround (for already-built apps)

The code fix only affects future builds. To unblock an existing crashing build:
**drag `AgentMux.app` into `/Applications` and launch it from there.**
