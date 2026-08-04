# Retro — "FD ownership violation" login crash is libcef's interposed `close()`, not Bun

**Date:** 2026-07-16
**Severity:** High (keystone auth blocker — every in-app provider login via
`runCliLogin` on Linux crashes the CLI before it can print an OAuth URL)
**Status:** Root-caused **and fixed** (this session); fix in `run_cli_login_pty`
**Component:** `agentmux-cef/src/commands/platform.rs` — `run_cli_login_pty`
**Supersedes:** the Bun-crash theory in `docs/specs/SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md` §2
and the memory note `login_again_fd_crash` (both attribute the crash to Bun's
startup fd-ownership guard).

---

## 1. Symptom

Clicking "Login Again" (or `/login`, or the auto-login phase of a fresh agent
launch) on Linux does nothing. The host log shows, ~1 ms after spawn:

```
[login-pty] Crashing due to FD ownership violation:
[login-pty] #0 … #1 … #2 close … (20-frame native stack)
run_cli_login_pty: no auth URL captured within 15s
run_cli_login_pty: child exited exit_code=1
```

`runCliLogin` returns `auth_url: null`, no browser opens.

## 2. What we previously believed (and why it was wrong)

`SPEC_HOST_CLI_LOGIN_CAPTURE` §2 and the `login_again_fd_crash` memory both
attributed the crash to **Bun's** startup fd-ownership guard tripping on the
CEF host's inherited fd table (sockets/eventfds). The evidence cited: the same
Claude binary runs fine from a plain shell, under `script(1)`, etc., and only
crashes when spawned by agentmux-cef.

That reasoning had the right *locus* (something about the CEF host's spawn) but
the wrong *mechanism*, and it led to dead-end fixes (setup-token capture, clean
fd tables) that couldn't work because the child never reaches its own `main`.

## 3. Actual root cause (proven live 2026-07-16)

The crash is emitted by **libcef.so**, not Bun. Chromium interposes a strong
`close` symbol (`base/files/scoped_file_linux.cc`) that guards against closing a
descriptor Chromium considers owned by a `ScopedFD`; on violation it
`IMMEDIATE_CRASH()`es with exactly this message and stack.

`portable_pty` 0.9's Unix spawn path (`unix.rs::spawn_command`) runs, in the
forked child's `pre_exec`, `close_random_fds()` — which calls the libc **`close`
symbol** on every fd ≥ 3. In this process that symbol resolves to libcef's
interposed `close`. The forked child still runs the host's memory image
(pre-exec), so it hits Chromium's guard and crashes **before `execve`** — which
is why no CLI, of any version or runtime, ever starts.

**Proofs:**
1. `nm -D libcef.so` shows `T close` (libcef exports a strong `close`), and the
   binary contains the literal string `Crashing due to FD ownership violation:`.
2. Driving `runCliLogin` against the **live** host (DevTools `Runtime.evaluate`
   → `window.api.runCliLogin`) with the target set to **`/bin/sh`** — not
   Claude — produced the **identical crash stack**. A shell has no Bun runtime,
   so Bun cannot be the cause. The crash is program-independent.
3. The crash addresses are byte-identical across runs and across target
   programs → the code running at crash time is the host image (pre-exec),
   confirming it's the parent's interposed `close`, not the child's `main`.
4. A standalone probe (`portable-pty` 0.9 + the pinned Claude 2.1.198, spawned
   from a binary that does **not** link libcef) ran cleanly even with a
   deliberately polluted 154-fd table and the full host environ — so neither
   the fd count nor the environment is the trigger. Only libcef linkage is.
5. `agentmux-srv` uses the same `portable_pty` 0.9 and never hit this: it
   doesn't link libcef, so its `close` is plain libc.

## 4. The fix

Replace `portable_pty`'s Unix spawn for the login child with a hand-rolled
`openpty` + `std::process::Command` + `pre_exec` (`spawn_login_pty_unix`) that
does its fd hygiene with the **raw `close_range(2)` syscall** instead of the
`close` symbol — `syscall(SYS_close_range, 3, U32_MAX, CLOSE_RANGE_CLOEXEC)`.
The raw syscall is invisible to symbol interposition, so Chromium's guard never
runs. `CLOSE_RANGE_CLOEXEC` marks fds ≥ 3 close-on-exec (they vanish at
`execve`) rather than closing them in-process, which also sidesteps the guard
entirely; it falls back to an outright close, then to leaving them (leaked fds
are harmless — proof #4). The pre-exec otherwise mirrors portable_pty (signal
reset, `setsid`, `TIOCSCTTY`). Windows keeps the ConPTY/portable_pty path
untouched via `#[cfg]`; a shared `LoginChildWait` trait keeps one reaper for
both. macOS uses per-fd `fcntl(FD_CLOEXEC)` (no `close_range`, and Chromium's
close guard is Linux-only anyway).

## 5. Verification

- `cargo check -p agentmux-cef` clean.
- New unit tests `spawn_login_pty_tests` (spawn+capture on a pty; child stdin is
  a TTY) pass — regression guard for the replacement spawn wiring.
- **End-to-end (the real proof):** rebuild the AppImage, drive
  `window.api.runCliLogin` against the pinned Claude CLI via DevTools, and
  confirm the `[login-pty]` capture shows the CLI's real output (URL / device
  code) instead of the FD-ownership crash. (Done in the same session against the
  freshly-built 0.53.6 host.)

## 6. Follow-ups / prevention

- **Grep guard:** anything spawned from agentmux-cef that runs a `pre_exec`
  calling `close`/`closefrom`/`close_random_fds` is a latent instance of this
  bug. Prefer `close_range(2)` (raw syscall) in any CEF-host child pre-exec.
- The frontend now also surfaces a visible error when `runCliLogin` returns no
  URL (PR "fix(auth): surface silent login-recovery failures…"), so even a
  future spawn regression can't present as a silently dead button.
- Update `login_again_fd_crash` memory and `SPEC_HOST_CLI_LOGIN_CAPTURE` §2 to
  point here for the corrected root cause.
