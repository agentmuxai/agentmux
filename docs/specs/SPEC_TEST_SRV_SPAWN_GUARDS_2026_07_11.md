# SPEC: Guard integration-test srv spawns (kill_on_drop / Job Object)

**Date:** 2026-07-11
**Status:** Ready for implementation
**Tracking:** session task #16
**Scope:** `agentmux-srv/tests/` (test infrastructure only — no product code)

## Problem

`agentmux-srv/tests/integration_test.rs::spawn_backend()` spawns a real
`agentmux-srv` subprocess via `std::process::Command::spawn()` and returns the
raw `Child`. Every test cleans up with an explicit `child.kill()` at the end of
its body — which never runs if any assertion between spawn and kill fails:
panic unwinds past the kill, the test harness reports the failure, and the srv
process **stays alive indefinitely** on the developer machine / CI runner.

Each leaked srv holds:
- two listening sockets (web + ws),
- an open SQLite store under the temp/test data dir,
- shell subprocesses it may have spawned for blocks.

On a dev machine this pollutes `tasklist` and can confuse instance discovery
(`muxlog ls`, wmic-based sweeps); on CI it accumulates until the runner
recycles. The E2E suites (`test/e2e/harness.ts`) already solved this class for
their instance spawns (wrapper-first kill + scoped wmic sweep in
`teardownInstance`); the Rust integration tests have no equivalent.

`subprocess_io.rs` spawns short-lived OS utilities as test *subjects* — those
exit on their own and are out of scope, but the audit in Phase 1 should confirm
none of them can block forever.

## Design

### Phase 1 — RAII kill guard (cross-platform, the actual fix)

Add a `KillOnDrop(std::process::Child)` wrapper in the test support code:

```rust
/// Kills the wrapped child on drop — including drop-during-panic-unwind,
/// which is exactly when the explicit end-of-test `kill()` never runs.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait(); // reap — avoid a zombie on unix
    }
}
```

`spawn_backend()` returns `(KillOnDrop, ...)` instead of a bare `Child`; tests
keep the guard alive for their duration and drop it (implicitly) at scope end.
The explicit `child.kill()` calls at the end of each test are deleted — the
guard owns cleanup unconditionally. `Deref`/`DerefMut` to `Child` keeps any
direct child access compiling.

Panic-unwind runs `Drop`, so a failed assertion now reaps the srv. This covers
every failure mode except the harness process itself being hard-killed.

### Phase 2 — Windows Job Object (hard-kill coverage, optional)

`Drop` cannot cover `cargo test` itself being terminated (Ctrl+C on some
shells, CI timeout SIGKILL, OOM-kill of the runner). On Windows, assigning the
child to a kill-on-close Job Object makes the OS reap it when the test process
dies, unconditionally:

- Small `#[cfg(target_os = "windows")]` helper in the test support module:
  create an anonymous Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`,
  `AssignProcessToJobObject` right after spawn, hold the job handle inside
  `KillOnDrop` so it closes (→ OS kills the child) no matter how the process
  exits.
- This mirrors what `agentmux-launcher/src/job_object.rs` does for production
  (J0); the test helper stays local to `tests/` rather than exporting the
  launcher's internals across crates — the API surface needed is ~15 lines of
  `windows-sys` calls. **Isolation invariants I2/I3 apply**: the job is
  anonymous (unnamed), created by the test itself, and can only ever contain
  processes the test spawned.
- Unix: process groups would be the analog; deferred — `Drop` + CI runner
  containerization already covers the realistic leak paths there.

## Definition of done

1. `spawn_backend()` returns a guard; no test in `integration_test.rs` calls
   `kill()` manually.
2. A deliberately-failing scratch test (assert right after spawn) leaves zero
   `agentmux-srv` processes behind — verified with `tasklist | grep` before/
   after on Windows.
3. (Phase 2) Killing the `cargo test` process mid-run leaves zero srv
   processes on Windows.
4. `subprocess_io.rs` audited: subject processes either self-terminate or get
   the same guard.
