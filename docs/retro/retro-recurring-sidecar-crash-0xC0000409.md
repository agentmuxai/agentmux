# Retro: Recurring Sidecar Crash — `0xC0000409` / `STATUS_STACK_BUFFER_OVERRUN`

**Date:** 2026-03-27
**Affected versions:** v0.32.73, v0.32.79, v0.32.84, v0.32.92 (confirmed), likely others
**Severity:** Critical — sidecar terminates silently, all terminal I/O and RPC lost

---

## Timeline of Today's Crash (v0.32.92)

| Time (UTC) | Event |
|------------|-------|
| 06:15:48 | agentmuxsrv-rs.exe started, v0.32.92 |
| 06:15:50 | npm install attempt for Codex CLI — fails ENOENT (separate path-quoting bug) |
| 14:13:33 | **Sidecar log ends abruptly** — last entry: `UpdateObjectMeta: 0.18ms` |
| 14:13:33 | **Host log ends** — last frontend entry: `raf-write chunks=1 bytes=3 bufLines=2181` |
| — | No `RUNTIME CRASH` log. No `backend-terminated` broadcast. No shutdown message. |

**Uptime at crash:** ~8 hours (06:15 → 14:13)

---

## What the Logs Tell Us

### Sidecar log — hard stop, no warning
```
{"timestamp":"2026-03-27T14:13:33.120510Z","level":"INFO","fields":{"message":"[http-perf] object.UpdateObjectMeta: 0.18ms"},...}
[EOF — no further entries]
```

No panic message. No `SIGTERM`. No `process exiting`. The process was healthy (serving HTTP requests with sub-millisecond latency) and then vanished.

### Host log — both stopped at the same second
The host log also ends at 14:13:33. No "Shutting down backend sidecar (last window closing)" — that message only appears on clean window close. The host either crashed separately, was killed, or simply stopped receiving events from the dead sidecar (the `raf-write` flood dried up immediately).

### Previous crashes — same pattern
From HANDOFF-2026-03-26.md:
> Seen in .73, .79, .84 (21:41, after 9.75hr uptime). Pattern: runtime crashes, not startup, high-load conditions suspected.

All are runtime crashes (not startup), all after multi-hour uptime, all with zero log output at the crash instant.

---

## Root Cause Analysis

### `0xC0000409` = `STATUS_STACK_BUFFER_OVERRUN` via `__fastfail`

This NTSTATUS code is emitted by `__fastfail(FAST_FAIL_STACK_COOKIE_CHECK_FAILURE)` — a compiler-generated security check. LLVM (Rust's codegen backend) emits these in several situations:

| Scenario | How triggered |
|----------|---------------|
| Stack cookie (`/GS`) check failure | Stack buffer overwrite detected on function return |
| `process::abort()` in some Rust stdlib paths | Double-panic, FFI boundary violation |
| Stack overflow on Windows | Translated to abort via alt-stack handler |
| Stack exhaustion in deeply recursive code | Tokio task overflow |

**The `__fastfail` path specifically:**
1. Compiler inserts `int 0x29` instruction
2. CPU traps to kernel `KiRaiseSecurityCheckFailure`
3. Kernel terminates process via `NtRaiseHardError` — **before returning to user mode**
4. No Rust panic hook. No `SetUnhandledExceptionFilter`. No VEH. No WER. No AeDebug. No log flush.

This is why the log ends mid-operation with no warning — the crash is instantaneous and uninterceptable from user mode.

### Why it's runtime, not startup

The crash happens after hours of uptime under load:
- v0.84: 9.75hr uptime
- v0.92: ~8hr uptime
- v0.73, v0.79: similar patterns

This points to **memory corruption that accumulates over time** — likely:
1. A heap buffer overrun in some long-lived data structure (block storage, WS message buffer, LAN discovery peer table)
2. A stack overflow in a deep Tokio async task chain under high load
3. A use-after-free in the filestore cache or sysinfo polling loop

The `UpdateObjectMeta` call pattern visible just before the crash (every ~1s) suggests the sysinfo/reactive subsystem was polling and writing to a block when the crash occurred.

---

## What We Know Is Not the Cause

- **Not the npm install failure** — that happened at 06:15:50, the crash was 8 hours later. The npm error was a separate bug (path quoting).
- **Not the WS reconnect** — PR #233 fixed that. The sidecar was running clean with no reconnect events.
- **Not a startup issue** — sidecar started cleanly, served requests for 8 hours.

---

## Evidence Gaps

1. **No crash dump** — `ForceDumpsEnabled` was configured *after* this crash. The next crash should produce a full heap dump at `C:\CrashDumps\agentmuxsrv\`.
2. **No callstack** — `__fastfail` bypasses WER on Windows 10 without `ForceDumpsEnabled`. Once we get a dump, WinDbg `!analyze -v` will show the faulting instruction and call stack.
3. **No `__fastfail` code** — the second parameter to `__fastfail` (which identifies the specific check that failed) is not logged anywhere. The dump will contain it in the exception record.

---

## What's Been Done

| Action | Status |
|--------|--------|
| WER `LocalDumps` key for `agentmuxsrv-rs.exe` | ✅ Configured |
| `ForceDumpsEnabled=1` in `CrashControl` | ✅ Applied via `apply-wer-fixes.bat` |
| WerSvc set to auto-start | ✅ Applied |
| Dump target folders created | ✅ `C:\CrashDumps\agentmuxsrv\` and `C:\CrashDumps\rustc\` |
| Sidecar crash recovery (Restart button) | ✅ PR #223 merged |
| WS reconnect after restart | ✅ PR #233 merged |
| Terminal resync after restart | ✅ PR #241 open |

---

## Next Steps (in priority order)

### 1. Get a callstack (blocker for everything else)
The next crash will produce `C:\CrashDumps\agentmuxsrv\agentmuxsrv-rs.exe.*.dmp`.

Open with WinDbg:
```
windbg -z C:\CrashDumps\agentmuxsrv\*.dmp
!analyze -v
k
~* k
```

Look for:
- The `__fastfail` code (second param to `KiRaiseSecurityCheckFailure`)
- The faulting thread's call stack
- Any heap corruption markers (`!heap -stat`, `!heap -s`)

### 2. Instrument the sysinfo / reactive poller
The crash consistently occurs while `UpdateObjectMeta` is being served — every ~1s, suggesting the sysinfo polling loop. Add bounds checks and buffer size assertions to:
- `agentmuxsrv-rs/src/backend/sysinfo.rs`
- `agentmuxsrv-rs/src/backend/reactive/poller.rs`
- `agentmuxsrv-rs/src/backend/storage/filestore/core.rs`

### 3. Add `SetUnhandledExceptionFilter` fallback
For non-`__fastfail` crashes (panics, access violations), add a `MiniDumpWriteDump` handler in `main.rs` using the `windows` crate. This catches crashes that WER misses due to race conditions.

### 4. Check for stack overflow in Tokio tasks
Run a debug build with `RUST_MIN_STACK=8388608` (8MB) to see if the crash disappears — would confirm a stack overflow hypothesis.

### 5. Review LAN discovery code (new in v0.32.92)
`lan_discovery.rs` was added in the version that crashed. It's a new networking subsystem with UDP sockets, peer tables, and periodic polling. Stack overflows or buffer issues in new code are a plausible cause of a runtime crash that only manifests after hours.

---

## Appendix: Crash Fingerprint

```
Version:   0.32.92
Uptime:    ~8 hours (06:15:48 → 14:13:33 UTC)
Exit code: 0xC0000409 (STATUS_STACK_BUFFER_OVERRUN / __fastfail)
Last log:  [http-perf] object.UpdateObjectMeta: 0.18ms
bufLines:  2181 (active terminal session with large scrollback)
Dump:      None (ForceDumpsEnabled not yet set at time of crash)
Pattern:   Matches .73, .79, .84 — runtime crash after multi-hour uptime
```
