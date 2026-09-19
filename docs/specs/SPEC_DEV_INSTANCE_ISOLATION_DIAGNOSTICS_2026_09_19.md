# SPEC: Fix misleading guidance when a second `task dev` instance collides

**Date:** 2026-09-19
**Status:** implemented — PR #3429.
**Related:** `docs/reports/REPORT_SCREENSHOT_TOOLING_ISOLATED_INSTANCE_SETUP_BLOCKERS_2026_09_19.md`
(Attempts 1-3 — the live repro this spec fixes), `docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md`
(the task that surfaced it).

---

## 0. The ask

Reproduced live this session: trying to run a second `task dev` instance
from the same branch, in the same clone, followed the tool's own suggested
fix (`task dev:local`) — a real ~7-minute rebuild — and it **still didn't
work**. The second instance's CEF process silently hung, then exited with
no explanation beyond `CEF early exit (process singleton or similar) —
exiting cleanly exit_code=24`. No data dir, no PID, no next step. This cost
roughly 20 minutes before the actual working fix (a different git branch/
worktree) was found by reading source code directly — something every
agent hitting this should not have to do.

## 1. Root cause (confirmed by reading the actual code, not guessed)

Two independent isolation axes exist in `task dev`, and they don't agree:

- **Launcher IPC socket** — `agentmux-launcher/src/hash.rs`'s
  `data_dir_hash16(data_dir, version)`, where `version` is
  `option_env!("AGENTMUX_BUILD_LABEL").unwrap_or(env!("CARGO_PKG_VERSION"))`
  — a **compile-time** constant. Bumping `Cargo.toml`'s version and
  rebuilding genuinely changes this key.
- **CEF profile / data directory** — `agentmux-common/src/data_paths.rs`'s
  `resolve_channel_and_dir`, called with `honor_env_channel=false` for any
  dev-mode launch (`agentmux-launcher/src/data_dir.rs`). This is
  **deliberate**, not a bug: dev mode's own comment says its data-dir
  isolation intentionally ignores `AGENTMUX_CHANNEL`. For
  `RuntimeMode::Dev{branch, ..}`, the resolved directory depends only on
  **git branch** (`agentmux-common/src/runtime_mode.rs`'s `detect_branch`,
  which re-runs `git rev-parse --abbrev-ref HEAD` itself) — no version
  component enters this path at all (`resolve_internal`'s `version_dir`
  is only version-scoped for `Installed`/`Portable`, not `Dev`).

So: `task dev:local`'s version bump changes the **first** axis (launcher
socket — genuinely produces a new socket, confirmed live) but not the
**second** (CEF's own `SingletonLock`, inside the branch-keyed data dir) —
which is the one that actually matters for "can two instances run at
once." **A same-branch second instance can never work via a version bump,
no matter how the version is bumped** — the only real fix is a different
branch (or a worktree on one).

Two places give the wrong impression that `task dev:local` is the fix:

1. `second_instance.rs`'s own refusal message: *"Use `task dev:local` to
   launch a second isolated session."*
2. `Taskfile.yml`'s `dev:local` task description and `scripts/dev-local.sh`'s
   own header comment, both of which describe its purpose primarily in
   terms of cargo-cache invalidation and a distinguishable version label —
   accurate for THAT purpose — but the refusal message's phrasing
   ("second isolated session") oversells it for this one.

Separately, `agentmux-cef/src/lib.rs`'s CEF early-exit log line has
`data_dir` in scope (logged two lines earlier, for a different purpose)
but doesn't include it in the exit message — so even someone who correctly
suspects a `SingletonLock` collision has no path/PID to go confirm it
against without re-deriving the directory from source.

## 2. Fix

Three small, targeted changes, no behavior change to the isolation model
itself (that's out of scope — see §3):

1. **`second_instance.rs`** (both call sites — the fast-path collision and
   the stale-socket-retry collision say the same thing): replace the
   `task dev:local` pointer with an accurate explanation — isolation is
   branch-keyed, a version bump doesn't help, use a different branch or
   worktree — and include the actual `data_dir` path in the message (it's
   already an argument to the function, just wasn't being printed).
2. **`agentmux-cef/src/lib.rs`**: the `exit_code` 0/24/36/38 early-exit log
   line now also logs `data_dir` (already in scope), so a silent
   second-instance hang is diagnosable directly from the log line instead
   of requiring a source read to know what to `fuser`/inspect.
3. ~~`Taskfile.yml` + `scripts/dev-local.sh`~~ — checked both directly:
   neither actually claims `dev:local` enables a second same-branch
   instance. `Taskfile.yml`'s own `dev:local` description already
   correctly frames it as cargo-cache invalidation + a distinguishable
   version label, and `dev-local.sh`'s header comment doesn't mention
   isolation at all. **The misleading claim existed only in
   `second_instance.rs`'s refusal message** (fix #1 above) — nothing to
   change here. Recorded so a future reader doesn't assume this file pair
   needs the same treatment based on the original bug report's phrasing.

## 3. Explicitly out of scope

- **Not** changing the actual isolation model (branch-keyed data dir,
  version-keyed launcher socket) — that's a bigger, riskier change
  (touches `agentmux-common`'s core path-resolution, used by every
  platform's dev/package/install flows) than "the error message lies."
  Fixing the message is the highest-leverage, lowest-risk win: it turns a
  ~20-minute source-reading detour into reading one corrected line.
- **Not** adding a PID-of-lock-holder lookup for the CEF `SingletonLock`
  collision — `data_dir` alone is enough to `fuser`/inspect manually, and
  Chromium doesn't expose the lock-holder PID through a stable API `agentmux-cef`
  already calls.
- **Not** the GPU-device-contention hang found in the same investigation
  (a separate, Linux-DRI-specific issue — see the report's Attempt 5) or
  the screenshot tool's own navigation-robustness gaps (Attempts 6-8) —
  tracked separately, lower confidence of cross-platform relevance for the
  former, different subsystem entirely for the latter.

## 4. Platform scope

The root-cause isolation logic (`hash.rs`, `data_paths.rs`,
`runtime_mode.rs`'s branch detection, the CEF early-exit handler in
`lib.rs`) is platform-agnostic — no `cfg(windows)`/`cfg(unix)` branch
governs any of it, so the underlying bug applies identically everywhere.
The **message fix** in `second_instance.rs` is Unix-specific
(`cfg(not(target_os = "windows"))`) because that's where the misleading
"`task dev:local`" phrasing actually lives — checked
`agentmux-launcher/src/supervisor/windows.rs`'s own second-instance path
directly: it logs and forwards (`open_new_window`) on a pipe-bind
collision but never makes the "use task dev:local" claim at all, so there
is nothing to fix on the Windows side for this specific issue.
