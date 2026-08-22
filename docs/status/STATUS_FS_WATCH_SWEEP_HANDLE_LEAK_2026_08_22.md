# Status: `FsWatchPool`'s Health Sweep Leaked a File + Semaphore Handle Pair Every Tick — RESOLVED

**Status: root cause confirmed source-level, fix applied and verified with a
live A/B (broken-vs-fixed) test on a real running `agentmux-srv` process.**

## 1. How this was found

Live-investigating an operator-reported "opening an old agent is slow, why
isn't it instant" complaint (same day, same host as
`docs/status/STATUS_CROSS_CHANNEL_AGENT_OPEN_FULL_APP_FREEZE_2026_08_22.md`).
That doc's leading hypothesis (a concurrent `output.idx` rebuild starving the
Tokio runtime) didn't hold up on review and was withdrawn there. A fresh live
repro on the same two long-running local instances showed **zero
`output.idx` rebuilds at all** during the slow open — ruling that mechanism
out for this repro — but the srv log's own `mem_attribution` watchdog fired
mid-stall: *"possible handle leak in an AgentMux process... handles: 51,917"*
(healthy baseline: under ~1K).

This looked at first like a recurrence of the already-fixed
`docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md` /
`STATUS_SRV_SECTION_HANDLE_LEAK_LIVE_RECURRENCE_2026_08_19.md` bug (sysinfo's
`CreateToolhelp32Snapshot` leak, fixed by PR #2666,
commit `3760067`) — but checking the actual handle-type breakdown
(`handle64 -s -p <pid>`) on the live process showed:

```
Section         : 11        <- flat, matches the FIXED signature exactly
File            : 25,751    <- NOT the sysinfo bug's fingerprint
Semaphore       : 25,715
```

`Section` sitting at the exact "fixed" baseline (11, matching the 08-19 doc's
own 4-hour soak-test result) confirmed the sysinfo fix is present and
working in this build — this is a **different, new, previously-undocumented
leak**, not a recurrence.

## 2. Root cause — confirmed, source-level

Full handle listing (`handle64 -a -p <pid>`) showed the File/Semaphore
handles concentrated on a small set of distinct paths, each with an
identical count (2,142) regardless of what the path was — every known
agent's memory directory (`~/.claude/projects/<hash>/memory`) *and* the
unrelated local-build channel's own `config` file, all at exactly the same
count. A uniform count across otherwise-unrelated paths, scaling with
process uptime, pointed at one shared, path-agnostic, periodically-ticking
mechanism rather than anything specific to memory-directory watching.

That mechanism is `FsWatchPool::sweep()`
(`agentmux-srv/src/backend/fs_watch/pool.rs`) — a background task that
re-issues `watch()` on **every currently-subscribed target**, unconditionally,
every `HEALTH_SWEEP_INTERVAL` tick (30s), to self-heal a watch that died
silently. The code's own doc comment stated this was safe: *"cheap (a
no-op for an already-live watch per `notify`'s own docs)"* — this claim was
wrong.

Reading the vendored `notify` 7.0.0 Windows backend directly
(`~/.cargo/registry/.../notify-7.0.0/src/windows.rs`, `add_watch`,
confirmed the current `main` branch of `notify` upstream still does the
same wasteful ordering, just no longer leaks — see §2.1):

```rust
fn add_watch(&mut self, path: PathBuf, is_recursive: bool) -> Result<PathBuf> {
    // ...
    handle = CreateFileW(/* opens a NEW directory handle, unconditionally */);
    // ...
    let semaphore = unsafe { CreateSemaphoreW(/* ... */) };
    // "every watcher gets its own semaphore to signal completion"
    // ...
    self.watches.insert(path.clone(), ws);  // <- silently OVERWRITES any
                                             //    existing entry for this
                                             //    path; the old WatchState's
                                             //    handle+semaphore are never
                                             //    passed to stop_watch() —
                                             //    orphaned, not closed.
    Ok(path)
}
```

`add_watch` has **no check for an existing entry before creating new
handles** — calling `watch()` twice on the same path opens two full native
watches (a `CreateFileW` directory handle + a `CreateSemaphoreW` completion
semaphore each) and the second call's `HashMap::insert` just drops the first
`WatchState` from the map with no cleanup — `stop_watch()` (which does
`CancelIo` + `CloseHandle` on both the directory handle and the semaphore)
is only ever called from `remove_watch`, which nothing here calls before
re-adding.

**Every piece of evidence matches exactly:**
- `sweep()` iterates *every* subscribed target uniformly, regardless of what
  it is — explains the identical count across unrelated paths (memory dirs,
  channel config).
- One redundant `watch()` per target per 30s tick, for the life of the
  process — 2,142 ticks × 30s ≈ 17.85 hours, a plausible uptime for the
  long-running local instance this was found on.
- One `CreateFileW` (File) + one `CreateSemaphoreW` (Semaphore) leaked per
  redundant call — matches the near-1:1 File:Semaphore ratio observed
  (25,751 : 25,715).

### 2.1 Is this already fixed upstream in `notify`?

Checked the current `notify` `main` branch (unreleased) directly. It *does*
add a cleanup step:

```rust
if let Some(ws) = self.watches.remove(&watched_path) {
    stop_watch(&ws, &self.meta_tx);
}
self.watches.insert(watched_path.clone(), ws);
```

but this cleanup still runs **after** creating the new `CreateFileW`/
`CreateSemaphoreW` handles, not before — so even the current upstream code,
if released, would still open a redundant handle pair as a transient step on
every redundant `watch()` call (briefly holding two live watches, then
dropping to one), not leak it permanently, but still wasteful — a repeat
`watch()` on an already-watched path was never actually the "cheap no-op"
this codebase's comment assumed, on any version checked. The fix applied
here doesn't depend on or wait for an upstream `notify` bump.

## 3. Fix applied

`sweep()` now calls `unwatch()` on each target **before** re-issuing
`watch()`, so at most one native watch per target exists at any instant —
regardless of how the underlying `notify` backend implements a redundant
`watch()` call. This preserves the silent-death self-healing `sweep()`
exists for (a genuinely dead watch is re-established exactly as before) at
the cost of a brief re-arm window every 30s per target — an already-accepted
class of imperfection in this system, since the fast-path/slow-path split
in `native_memory_drift.rs` already treats a missed fs-watch event as normal
and backstops it with its own 30s reconciliation sweep.

Also corrected the "cheap no-op" claim in three places it appeared:
`pool.rs`'s `sweep()` doc comment, `pool.rs`'s health-sweep task spawn
comment (`FsWatchPool::new()`), and `recovery.rs`'s `HEALTH_SWEEP_INTERVAL`
doc comment.

## 4. Verification

New regression test,
`fs_watch::pool::tests::sweep_does_not_leak_a_handle_pair_per_call`
(Windows-only): subscribes to a real temp directory, calls `sweep()` 200
times back-to-back, and asserts this process's own `GetProcessHandleCount`
(reusing `backend::sysinfo::process_handle_count`, the same helper the
sysinfo-leak regression test uses) doesn't grow linearly.

**Proved the test is a real discriminator, not just a green checkmark**
(same methodology the 08-19 sysinfo fix used): temporarily reverted just the
`unwatch()`-before-`watch()` change (kept the test), reran —

```
handle count grew by 400 over 200 sweep() calls on one subscribed target
(before=155, after=555)
```

— **exactly 2.0 handles/call**, matching the theorized File+Semaphore pair
precisely. Restored the fix, reran clean. Full suite:
`cargo test -p agentmux-srv -- --test-threads=1` — 2639 passed, 0 failed.

## 5. What this does NOT fix — action needed on already-running instances

Same caveat as the 08-19 sysinfo fix: **this is a code fix, not a live
mitigation.** The two long-running local instances this was found on
(`0.55.18` PID 67608, `0.55.19` PID 2332) will keep leaking at their current
rate until they're rebuilt/restarted with a build that includes this fix —
their already-leaked handles do not self-heal. Confirmed no correlation
found (or claimed) between this leak and the separate, still-unresolved
`output.idx`-rebuild-attribution question in
`STATUS_CROSS_CHANNEL_AGENT_OPEN_FULL_APP_FREEZE_2026_08_22.md` §4 — that
doc's causal claim was withdrawn independently of this finding; this leak is
additive to whatever else makes a cross-channel agent open slow, not a
replacement explanation for it.

## 6. Sources

- `agentmux-srv/src/backend/fs_watch/pool.rs` (`sweep()`, `FsWatchPool::new()`)
- `agentmux-srv/src/backend/fs_watch/recovery.rs` (`HEALTH_SWEEP_INTERVAL`)
- `agentmux-srv/src/backend/native_memory_drift.rs` (the consumer whose
  memory-directory watches surfaced this — not itself the bug)
- `agentmux-srv/src/backend/sysinfo.rs` (`process_handle_count`, reused for
  the regression test)
- `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/notify-7.0.0/src/windows.rs`
  (`add_watch`, `stop_watch`, `remove_watch` — read directly, not from docs)
- `docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md`,
  `docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_LIVE_RECURRENCE_2026_08_19.md`
  (the prior, unrelated, already-fixed leak this was initially mistaken for)
- `docs/status/STATUS_CROSS_CHANNEL_AGENT_OPEN_FULL_APP_FREEZE_2026_08_22.md`
  (the investigation this fix branched off from)
