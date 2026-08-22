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

## 3. Fix applied — revised after PR review (codex P2 on #2722)

**First version** (superseded, kept here for the record): `sweep()` called
`unwatch()` on *every* target before re-issuing `watch()`, every tick — no
leak, but review correctly caught that this opened a real coverage gap.
`unwatch()` then `watch()` leaves the target genuinely unwatched for the
gap between the two calls. `native_memory_drift`'s slow-path reconciliation
sweep backstops a missed event there, but three other consumers of the same
pool — `config_watcher_fs`, `EditorFileWatcher`, `MediaFileWatcher` — rely
solely on the broadcast stream with no equivalent backstop. A create/modify
landing in that ~instant gap, on any watched path, every 30 seconds, for
the life of the process, would go unnoticed until some later, unrelated
change on the same path happened to trigger a refresh.

**Fix, revised**: `sweep()` now only re-arms targets that are already
*degraded* (`HealthState::mark_degraded` was called for them) — a
healthy-looking target is left completely untouched, every tick. This
fixes the leak the same way (nothing is ever double-watched) *and* fixes
the coverage gap (a healthy watch is never interrupted).

**A second review round (reagent P1) caught that this trade-off was
initially stated too narrowly.** The bridge task that forwards `notify`'s
raw callback into the broadcast stream had an error branch that only
logged (`tracing::warn!`) — it never called `mark_degraded`. Before the
degraded-only scoping, that didn't matter: the unconditional per-tick sweep
re-armed every target regardless, so an untracked error still got fixed on
the next tick. After the scoping, it did matter — a watch that died with an
*explicit, non-silent* `notify` error (e.g. a Windows
`ReadDirectoryChangesW` buffer-overflow, which `notify` surfaces as a real
`Err` carrying the affected path via `notify::Error::paths`) would never be
marked degraded, so `sweep()`'s `is_degraded()` filter would never pick it
up either — a real, broader-than-documented regression, not just the
narrow "truly silent, never-erroring death" case the first revision
described. Fixed by having that error branch call
`health.mark_degraded(path, ...)` for every path `notify::Error::paths`
attributes the error to (logged either way; a path-less error still can't
be attributed to a specific target, so it's logged only).

**The trade-off, now accurately scoped**: only a watch that dies *without
any reported error at all* — no explicit `notify::Error`, no `watch()`
failure, truly silent — is unhealed by this sweep. Every explicit failure
this pool can observe (initial `watch()` failure, sweep-time `watch()`
failure, or an async `notify` error with an attributable path) now
degrades the target and gets re-armed on the next tick. Judged acceptable
because a *truly* silent death (no signal of any kind) has no confirmed
occurrence in this codebase's history (the scenarios
`HEALTH_SWEEP_INTERVAL`'s own doc comment lists — inotify instance-limit
churn, flaky network mounts — are Linux-flavored concerns on a
Windows-primary codebase), against a certain, systematic cost (leak or gap,
every target, every tick) the unconditional-sweep alternative imposed on
every watch.

Also corrected the "cheap no-op" claim in three places it appeared:
`pool.rs`'s `sweep()` doc comment, `pool.rs`'s health-sweep task spawn
comment (`FsWatchPool::new()`), and `recovery.rs`'s `HEALTH_SWEEP_INTERVAL`
doc comment — all now describe the degraded-only scoping and why.

### 3.1 A third review round (reagent P1) — snapshot/act race with `unsubscribe()`

`sweep()`'s per-target loop snapshotted the degraded-target list once
under the pool's lock, then released the lock and acted on each target in
turn (`unwatch()` + `watch()`). If a concurrent `unsubscribe()` dropped the
last subscriber for one of those targets during that window, it would
remove the target from `targets` (and `unwatch()` it itself) — but
`sweep()`'s own in-flight action for that same target would still go on to
call `watch()` afterward, re-establishing a native watch for a path with
no subscriber left and no future sweep tick able to see it again (it's no
longer in `targets`, so `is_degraded()`'s target list never includes it).
That orphans the handle pair permanently — worse than the original bug in
one respect, since this one can never self-heal even under continued
normal operation.

**Fix**: extracted `sweep()`'s per-target action into its own method,
`rearm_if_still_subscribed(&self, target: PathBuf)`
(`agentmux-srv/src/backend/fs_watch/pool.rs`), which re-checks target
membership and performs `unwatch()`/`watch()` under a single held lock —
not released between the check and the act, and not implemented by
calling back into a helper that re-locks (`std::sync::Mutex` isn't
reentrant, so that would deadlock). A target removed by `unsubscribe()`
before `rearm_if_still_subscribed` acquires the lock is simply skipped —
`sweep()` no longer resurrects a watch nobody wants anymore.

### 3.2 A fourth review round (reagent P2) — the round-2 fix itself was never actually exercised

All three sweep regression tests (§4) simulate degradation by calling
`pool.health.mark_degraded(...)` directly — none of them drive a real
`notify::Error` through the bridge task's error branch (§3's round-2 fix).
That left an unverified assumption the whole round-2 fix depends on:
`notify::Error::paths` has to line up, path-for-path, with the exact
`PathBuf` keys `inner.targets` uses (the canonicalized `watch_target` a
subscription produces), or `mark_degraded` records a path that
`is_degraded()` will faithfully report as degraded while `sweep()`'s own
`targets.keys().filter(is_degraded)` never matches it — silently defeating
the fix for any real backend error, with no test able to catch it because
every test bypassed exactly the part in question.

**Fix**: extracted the bridge task's error-handling branch into its own
method, `handle_backend_error(&self, e: notify::Error)`
(`agentmux-srv/src/backend/fs_watch/pool.rs`), so a test can drive a real
`notify::Error` (`notify::Error::generic(...).add_path(...)`) through it
directly using a real subscription's `watch_target` — the same
`canonicalize()`-derived path a live `notify` callback's error would need
to match — rather than a hand-built one.

## 4. Verification

Four regression tests in `fs_watch::pool::tests` (all Windows-only except
`handle_backend_error_degrades_the_exact_target_key_sweep_looks_up`, which
doesn't touch a real OS watcher and runs on every platform):

- `handle_backend_error_degrades_the_exact_target_key_sweep_looks_up`
  (§3.2's fix) — subscribes to a real temp directory, constructs a real
  `notify::Error` carrying that subscription's own `watch_target`, drives
  it through `handle_backend_error` directly, and asserts
  `health.is_degraded()` returns true for that *exact* `PathBuf` —
  confirming the path threading actually works end-to-end rather than
  only through directly-called `mark_degraded`.

- `sweep_leaves_a_healthy_target_untouched` — subscribes to a real temp
  directory, calls `sweep()` 200 times back-to-back on the now-healthy
  target, and asserts this process's own `GetProcessHandleCount` (reusing
  `backend::sysinfo::process_handle_count`, the same helper the sysinfo-leak
  regression test uses) doesn't grow at all — proving a healthy watch is
  genuinely skipped, not just "skipped but still leak-free by luck."
- `sweep_does_not_leak_a_handle_pair_per_call_for_a_degraded_target` —
  forces the same target into `degraded` state before every one of 200
  sweep calls (so each one really exercises `unwatch()` + `watch()`), and
  asserts handle count doesn't grow linearly — proving the re-arm path
  itself, which does still run for real failures, doesn't leak either.
- `sweep_does_not_resurrect_a_target_unsubscribed_after_being_snapshotted`
  (§3.1's fix) — deterministically reproduces "sweep already decided to
  process this target, then it was removed before the action ran" by
  calling `rearm_if_still_subscribed` directly with a target captured
  *before* a real `unsubscribe()` runs, rather than trying to race two
  tasks against each other (an earlier version of this test tried a real
  `tokio::spawn` race and found it unreliable — `unsubscribe()`'s critical
  section is far shorter than the scheduling latency needed to land it
  mid-`sweep()`, so the two operations essentially never interleaved).
  Repeats the whole subscribe → degrade → unsubscribe → single-stale-rearm
  sequence across 200 distinct targets rather than looping one target
  200 times — an early draft of this test that reused a single target
  didn't discriminate, because each iteration's own `unwatch()` silently
  cleaned up the *previous* iteration's orphaned watch, masking the leak.
  A single-target, single-round before/after diff didn't discriminate
  either — too small to separate from ambient handle-count noise. Only
  amplifying the one-time-per-target orphan across many distinct targets
  produced a reliably measurable total.

**Proved all three are real discriminators, not just green checkmarks**
(same methodology the 08-19 sysinfo fix used): for the first two, temporarily
reverted just the `unwatch()`-before-`watch()` line (kept both tests, kept
the degraded-only filter), reran the degraded-target test —

```
handle count grew by 400 over 200 sweep() calls on a degraded target
(before=153, after=553)
```

— **exactly 2.0 handles/call**, matching the theorized File+Semaphore pair
precisely. For the third (§3.1's fix), temporarily reverted
`rearm_if_still_subscribed`'s re-check (using a hardcoded `RecursiveMode`
instead of looking the target up), reran —

```
handle count grew by 400 over 200 distinct subscribe -> unsubscribe ->
stale-rearm rounds (before=151, after=551)
```

— again **exactly 2.0 handles/call**, the same signature. Restored the fix
each time, reran clean. Full suite: `cargo test -p agentmux-srv --bin
agentmux-srv -- --test-threads=1` — 2641 passed, 0 failed.

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
