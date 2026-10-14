# Retro: an AgentMux instance quit by itself when a signal reached srv's launcher watcher

**Date:** 2026-10-14
**Author:** Opaz
**Status:** implemented: srv's launcher watcher retries an interrupted wait, and
falls back to the ppid watchdog on any other wait error instead of exiting
**Severity:** High when it hits: the whole instance (every window and every agent
running in it) quits without warning. Rare: one occurrence in the launcher logs
of a Linux dev host. Linux and macOS; Windows uses a different mechanism.

---

## 1. Symptom

On 2026-10-14 at 06:28:19 UTC, a packaged AgentMux 0.59.23 (AppImage, Linux,
GNOME Wayland) closed with nobody touching it. The operator came back to find it
gone; the agents running in it were gone with it. Nothing looked like a crash:

- The host and srv logs both stopped mid-stream, at 06:28:18.8 and 06:28:18.7,
  with no shutdown lines. The host had been logging about once a second.
- No core dump, no segfault, no out-of-memory kill in the kernel log.
- The launcher's event log recorded the host's exit as `code: 0`.
- The CEF process scope ended at 06:28:19, and the AppImage was unmounted 5 s
  later.

Because a dev build of another agent was being compiled on the same machine at
the time, the first suspicion was that something had killed the instance. No
agent transcript on the machine had a kill, quit or AppImage command in that
window, and `task dev`'s only kill path (reaping an orphaned Vite on its own
port) runs later and only matches the clone's own `node`.

## 2. What happened

The launcher's own log (`~/.agentmux/logs/agentmux-launcher.log`, which
captures srv's stderr) has the sequence, all within the same second:

```
[srv stderr] poll() on pidfd returned -1 (errno=4), shutting down
notify: srv connection ended (... Connection reset without closing handshake)
srv exited as part of shutdown — shutting down
stopping children (host: SIGTERM → grace → SIGKILL; srv: quiesce)
[ipc] connection conn_id=1 closed (kind=Some(Host), pid=Some(...))
srv stopped (AlreadyExited)
launcher exiting with code 0
```

1. srv watches its parent, the launcher, so that it shuts down when the launcher
   goes away (`bootstrap/watchers.rs`, `start_parent_watcher`). On Linux a
   dedicated thread opens a pidfd for the launcher and blocks in `poll()` on it.
2. `poll()` returned -1 with `errno = 4`, which is `EINTR`: a signal handler ran on
   that thread while it was waiting. The launcher was alive.
3. The watcher treated every result other than "pidfd readable" as the launcher
   being gone, and called `std::process::exit(0)`.
4. The launcher saw srv exit, treated it as part of a shutdown, stopped the host
   and exited. The host and srv run with `PR_SET_PDEATHSIG = SIGKILL`, so nothing
   was left to log a reason.

The launcher's `code: 0` for the host is not evidence of a clean quit: the
launcher writes `code: 0` for any host disconnect (`dispatch_synthetic_goodbye`
→ `handle_goodbye`), graceful or not.

## 3. Root cause

`poll()` is never restarted after a signal handler runs, whatever `SA_RESTART`
says (signal(7), "Interruption of system calls and library functions by signal
handlers"). The watcher did not retry on `EINTR`.

srv catches several signals. The running srv's `SigCgt` mask decodes to SIGINT,
SIGTERM, SIGBUS, SIGSEGV and SIGCHLD. SIGINT and SIGTERM shut srv down anyway,
and SIGBUS/SIGSEGV are crashes, so the realistic trigger is **SIGCHLD**: srv runs
many child processes (shells, agent CLIs, probes), and every one that exits sends
srv a SIGCHLD. A process-directed signal is delivered to any one thread that
doesn't block it. Most of the time that is a thread other than the watcher, which
is why this is rare; when it is the watcher, srv quits. The log does not record
which signal it was.

The macOS watcher has the same flaw: its blocking `kevent()` wait also returns
`EINTR` when interrupted, and it also exited on any result other than an event.

The bug is old. #4721 (2026-10-12) touched these lines only to swap `eprintln!`
for `eprintln_lossy!`; the exit-on-any-error logic predates the split of
`bootstrap.rs` into `bootstrap/` (#4061).

## 4. Fix

- **Linux:** the wait moved into `wait_for_pidfd_exit(pidfd)`, which loops until
  the pidfd is readable, retrying on `EINTR` (and on a 0 return, which an
  infinite timeout should never give).
- **macOS:** the `kevent()` wait loops the same way.
- **Any other wait error** no longer shuts srv down. A failed wait says nothing
  about the launcher, so the watcher logs it and falls back to the existing ppid
  watchdog, which polls `getppid()` every 2 s. Only a real exit event shuts srv
  down.
- **The ppid watchdog takes the launcher's pid from its caller** instead of
  reading `getppid()` when it starts, and checks once before its first sleep. A
  fallback that starts after the launcher already died (srv reparented) would
  otherwise take the new parent as its baseline and never fire.

## 5. Test

`bootstrap::watchers::tests::a_signal_does_not_end_the_wait_only_the_process_exiting_does`
(Linux): installs a SIGUSR1 handler without `SA_RESTART` (as srv's SIGCHLD is to
`poll()`), waits on the pidfd of a `sleep 60` child in a thread, sends that thread
SIGUSR1 ten times, and checks that the wait is still going. It then kills the
child and checks that the wait ends with `Ok`.

With the retry removed, the test fails with the production error:
`the wait ended while the process was alive: Ok(Err("Interrupted system call (os error 4)"))`.

The macOS loop has no test of its own (no macOS runner here); it is the same
pattern as the Linux helper.

## 6. Lessons

- A blocking syscall in a process that catches signals must handle `EINTR`.
  "Returned an error" and "the thing I'm waiting for happened" are different
  outcomes; only the second should trigger an irreversible action like exiting.
- When one process's exit takes others down (launcher → host via
  `PR_SET_PDEATHSIG`), the root cause is logged by the first process to go. Here
  the host and srv logs were silent and the answer was in the launcher's log,
  which carries srv's stderr.
- `code: 0` in the launcher's event log does not mean a clean exit.
