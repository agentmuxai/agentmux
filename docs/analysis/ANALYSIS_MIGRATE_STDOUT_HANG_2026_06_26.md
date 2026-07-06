# Analysis: Migrate Subprocess Stdout Hang — Splash Stuck at "Migrations"

**Date:** 2026-06-26  
**Version affected:** 0.49.4 (first observed; likely present in prior versions)  
**PR:** #1797 (`fix/migrate-stdout-hang`)  
**Status:** Fixed

---

## Symptom

After launching the 0.49.4 portable build, the splash screen showed "Migrations" as the active stage for 44+ seconds (and indefinitely thereafter). The app never progressed to "Backend startup" or first paint.

---

## Root Cause

`run_migrate` in `agentmux-launcher/src/srv_spawner.rs` reads the migrate subprocess's stdout in an inline loop until EOF:

```rust
while let Ok(Some(line)) = reader.next_line().await {
    // parse migration JSON events...
}
// then: child.wait().await
```

The migrate subprocess (`agentmux-srv --migrate`) emits all its events — including the final `{"event":"complete"}` — then **stalls in Tokio runtime shutdown** instead of exiting. The shutdown stall is caused by the crash-monitor task:

1. On startup, the srv binary installs a VEH crash handler and spawns a `--crash-monitor` subprocess (confirmed as pid 190372, `agentmux-srv-0.49.4-windows.x64.exe --crash-monitor`)
2. The crash monitor holds an async handle (likely `WaitForSingleObject` or a named-pipe connection via `monitor.sock`) that never resolves
3. Tokio's graceful runtime shutdown waits for all tasks to complete — this task never does
4. The migrate subprocess is therefore **permanently alive** after printing "complete"
5. With the process alive, its stdout pipe stays open — **EOF never arrives**
6. The launcher's `reader.next_line().await` blocks forever waiting for the next line
7. `stage_end("migrations")` is never called → splash stays on "Migrations"
8. `spawn_srv` is never called → no backend → no app

---

## Evidence Trail

| Observation | Implication |
|---|---|
| `[migrate] {"event":"complete","applied":4,"skipped":9}` appears in launcher log at `[1782456947]` | Migrate subprocess emitted complete event — migrations succeeded |
| `[1782456947]` is the LAST 0.49.4 entry in launcher log | Launcher never advanced past `run_migrate` |
| `[ipc] srv pipe path = ...` (logged immediately after `run_migrate`) is absent | Code never reached line 1671 in `main.rs` |
| `agentmux.exe` pid 275300 alive at 17MB | Launcher process did not exit |
| `agentmux-srv-0.49.4-windows.x64.exe` pid 190372 alive at 11MB | Crash monitor still running |
| `Get-CimInstance` on pid 190372: `CommandLine = ... --crash-monitor` | Confirmed crash monitor, not migrate subprocess |
| No other `agentmux-srv-0.49.4` in process list | Migrate subprocess already exited OR is hidden |
| All db files timestamped `Jun 25 23:55` (migrations moment) | No db activity since migrations — srv never started |
| `launcher-sagas.db-wal` at 41KB | Launcher progressed far enough to write saga data |

The crash monitor (pid 190372) staying alive 12+ minutes after migrations completed is the key signal: the crash monitor only exits when the process it monitors exits. Since the crash monitor is still alive, the migrate subprocess is still alive (or was alive very recently), confirming the hang is in the migrate process's shutdown.

---

## Fix

**`agentmux-launcher/src/srv_spawner.rs`**

### 1. Break on `Complete`

```rust
Some(MigrationLine::Complete { applied: a, skipped: s }) => {
    applied = a;
    skipped = s;
    migration_complete = true;
    break; // Don't wait for EOF — kill below.
}
```

Stop reading stdout as soon as the complete event arrives. The subprocess may still be running, but we have everything we need.

### 2. Force-kill before wait

```rust
// If we got a complete event, kill the migrate process so we don't hang
// on wait() if its Tokio runtime shutdown stalls.
if migration_complete {
    let _ = child.start_kill();
}

let status = child.wait().await...;
```

`start_kill()` sends `TerminateProcess` on Windows. The process exits within milliseconds. `wait()` returns immediately.

### 3. Treat stdout signal as authoritative

```rust
if migration_complete || status.success() {
    // success path
}
```

A force-killed process returns a non-zero exit code. Since we killed it ourselves after seeing the success signal, we use `migration_complete` as the authoritative indicator. `status.success()` is only the tiebreaker for the case where the process exited naturally before we could kill it.

---

## Why This Was Silent Before

The crash monitor interaction is not new — it exists in prior versions. However, the hang may have been masked in earlier builds by:

- Faster SQLite shutdown timing on less-loaded databases (newly migrated db has more WAL to checkpoint)
- The `dedup_identity_accounts` migration deleting **100 rows** — a large write that creates substantial WAL entries, potentially extending the SQLite teardown that keeps the srv binary's I/O threads alive longer
- Previous migration runs on established databases applying 0 migrations (skipped all), meaning the srv in migrate mode exited very quickly with minimal cleanup

The first time 0.49.4 ran against an existing database, all 4 pending migrations applied (including the 100-row deletion), and the subsequent shutdown took long enough to trigger the hang reliably.

---

## Related Gaps (from Splash Telemetry Audit)

This hang exposed a deeper architectural gap in the splash telemetry: there is no per-stage timeout. The spec (§9) mandates a `⚠` prefix on the running clock after 10 seconds, but this is not implemented. Had the warning been present, the user would have seen the stage flag at 10s, making the hang immediately diagnosable without log access.

Recommended follow-up:
1. Add `⚠` prefix to running stages after 10 seconds (spec §9)
2. Add a hard timeout to `run_migrate`'s `child.wait()` as a second defense (e.g., 60s with force-kill and error)
3. Investigate whether the crash monitor subprocess should be suppressed in `--migrate` mode — a crash during a migration run needs a dump, but the monitor's async shutdown cost is now a known startup hazard

---

## Timeline

| Time (UTC) | Event |
|---|---|
| 06:55:46 | 0.49.4 launcher started (pid 275300) |
| 06:55:46 | IPC server, saga coordinator initialized |
| 06:55:46 | Migrate subprocess spawned |
| 06:55:47 | All 4 migrations complete (total ~50ms) |
| 06:55:47 | `{"event":"complete"}` emitted — LAST launcher log entry for 0.49.4 |
| 06:55:47+ | Migrate subprocess stalls on Tokio shutdown |
| 06:55:47+ | Crash monitor (pid 190372) remains alive watching migrate process |
| 06:55:47+ | `run_migrate` stdout loop blocks waiting for EOF |
| ~07:08+ | User reports hang; diagnosis begins |
| ~07:10 | Root cause identified via process list + command-line inspection |
| ~07:12 | Fix implemented and compiled |
| ~07:14 | PR #1797 opened |
