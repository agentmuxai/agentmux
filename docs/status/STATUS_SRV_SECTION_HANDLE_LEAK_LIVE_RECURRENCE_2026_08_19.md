# Status: Live Recurrence of the `agentmux-srv` Section Handle Leak — Windows 11 (2026-08-19)

**This is a follow-up to `docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md`
(still open, unpatched).** Today's investigation started from a user report
framed as "AgentMux causes growing Page File pressure on Windows 11; Windows
10 doesn't have it" and a request to check repo history. This doc (a) answers
the Win10-vs-Win11 framing directly, (b) reports a live, current-day
reproduction of the exact 2026-08-08 fingerprint on this machine, and (c)
consolidates the full history across all three related investigation threads
so this doesn't need re-discovering again.

> **Update, same day — ROOT CAUSE CONFIRMED, source-level.** §7 below pins
> the exact bug: the vendored `sysinfo` crate v0.34.2 (agentmux-srv's pinned
> version) never closes the Windows handle it obtains from
> `CreateToolhelp32Snapshot` in its `refresh_processes_specifics` — on any
> code path — and that handle is backed by a kernel `Section` object. Fixed
> upstream between sysinfo v0.34.2 and v0.35.0 (confirmed by reading both
> versions' source directly). **No code archaeology or live tracing was
> needed to close this — it's a one-line-summary dependency bug: bump
> `sysinfo` to ≥0.35.0 (latest stable: 0.39.6).** See §7 for full evidence
> chain and §8 for the recommended fix.

**Status: RESOLVED, same day.** Root cause confirmed source-level (§7), fix
applied and merged (§8, PR #2666, commit `3760067`), and confirmed with a
4-hour live production soak test on a real build of the fix (§9): `Section`
handles held flat at 11 for the entire run while total handles/memory grew
normally from ordinary activity. This closes out both this doc and
`STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md`. The live, already-leaking
production instance investigated in §3 (PID 30088, 500K+ Section handles at
the time) was never restarted as part of this work — it still needs a
normal restart/update to pick up the fix, same as any other change; its
existing leaked handles do not self-heal without that.

## 1. Does Windows 11 specifically leak, and Windows 10 doesn't?

**No evidence of an OS-version-gated code path exists, in either direction.**
`git grep` across `agentmux-srv` and `agentmux-cef` for `IsWindows11OrGreater`,
`IsWindows10OrGreater`, `RtlGetVersion`, `dwBuildNumber`, or any OS-version
branch returns **zero matches** — the app has no Windows-version-conditional
logic at all. The one place in the repo that reasons explicitly about a
Win10/Win11 split, `docs/specs/SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`,
found the **opposite** direction (a Windows 10 machine hit OOM-abort crashes
from a *shrunk* pagefile ceiling) and its own conclusion (§4) was explicit:
*"Not a kernel-version bug... the dominant lever is environmental (free disk
→ pagefile growth)."*

The most likely explanation for the user's own Win10-clean / Win11-affected
observation: the leak mechanisms found below scale with **process uptime and
usage pattern** (handle count climbs continuously the longer `agentmux-srv`
runs), not OS version — two machines with different typical session lengths,
pane counts, or background-service topology will show very different symptom
severity by coincidence, independent of which Windows version they run. The
one *confirmed, fixed* Windows-11-specific bug in the repo
(`docs/retro/retro-windows-terminal-window-leak-2026-06-21.md` — missing
`CREATE_NO_WINDOW` caused Win11's DefTerm to pop a visible Terminal window
per CLI/LSP spawn) is real and Windows-11-specific, but it leaks **window
handles/UX clutter**, not commit/page-file — a different mechanism, already
fixed in commit `b76ecd1e4`.

## 2. Full history (three independent threads, chronological)

1. **2026-07-02 → 2026-07-16 — GPU/driver renderer-pool commit.** Every pane's
   isolated Chromium `RequestContext` costs the GPU driver ~1 GB of invisible,
   kernel-charged, pagefile-backed commit. Confirmed *not* a leak (07-16:
   closing AgentMux instantly released 43.6 GB) — it's a live floor that
   ratchets up because pooled renderers weren't torn down on pane close.
   Tracked as issue **#2218**; fixed by PRs **#2220/#2221/#2222** (shipped
   v0.54.4). `--disable-gpu` was explicitly rejected as a mitigation (owner
   policy: GPU stays enabled). See `docs/retro/retro-commit-charge-pagefile-growth-2026-07-02.md`
   and `docs/retro/retro-commit-restart-reclaim-2026-07-16.md`.
2. **2026-07-24 → 2026-07-25 — Audiosrv (OS-level, unrelated to AgentMux).**
   A ~55 KB, 16-section investigation (`docs/status/STATUS_PF_COMMIT_GROWTH_INVESTIGATION_2026_07_24.md`)
   chased ~800 MB/hour of unattributed commit growth through several dead
   ends (including reproducing an apparently-identical pattern in plain MS
   Edge) before finding, via a handle-count sweep, that Windows' own
   `Audiosrv` (Windows Audio service) was leaking ~1 pagefile-backed
   `Section` handle/sec (~17 GB/day) — invisible to any per-process memory
   view because Sections aren't charged to a process's private bytes.
   `Restart-Service Audiosrv` reclaimed 16.4 GB in ~9 seconds. **AgentMux was
   fully exonerated** in this thread. Likely trigger: an always-on audio
   interface (unconfirmed, never conclusively pinned, judged out of
   AgentMux's scope). This doc is also where the Windows-vs-macOS framing was
   first raised (§4) as "highest-value open thread" — it was never actually
   executed before the investigation resolved via the Audiosrv path instead.
3. **2026-08-08 → 2026-08-09 — `agentmux-srv` itself leaking Section handles.
   STILL OPEN.** This time the leak is inside AgentMux's own backend process,
   not an OS service — same fingerprint as Audiosrv (near-100% anonymous
   `Section` handles) but a different, unpinned cause. See §3 below: this is
   what reproduced live today.

## 3. Live reproduction, this machine, today (2026-08-19)

- **System commit: 2149 MB free / 186,899 MB total — 98.9% used**, confirmed
  two ways: raw `Win32_OperatingSystem`/`Win32_PageFileUsage` WMI counters,
  and AgentMux's own `muxlog mem` doctor (agrees: "98.9% used → ok" — worth
  noting the tool doesn't yet flag this level as a problem itself).
- Two `agentmux-srv` instances were running side by side, different versions
  and uptimes — a natural A/B pair:

  | PID | Build | Started | Total handles | `Section` handles | Threads | Private MB |
  |---|---|---|---|---|---|---|
  | 30088 | 0.55.10 | Sun Aug 16 | **517,490** | **500,106 (96.6%)** | 70 | 267 |
  | 74924 | 0.55.15 | Tue Aug 18 | 8,662 | 2,817 (32.5%) | 53 | 161 |

  (Full `handle64 -s` breakdown for PID 30088: Section 500,106, File 8,281,
  Semaphore 8,231, Process 355, Thread 160, Event 117, everything else
  small. GDI/USER object counts for both `agentmux-srv` PIDs are ~0/1 —
  this is **not** a GDI or window-handle leak, ruling out any connection to
  the fixed Terminal-window bug from §1.)
- **This is the identical fingerprint to the 2026-08-08 finding**
  (`agentmux-srv-0.54.10`, PID 45728, 215,705 handles, 99.5% Section) —
  same process, same handle type, same near-total-anonymous pattern — just a
  newer version (0.55.10 vs 0.54.10) and a higher absolute count (11 days of
  no fix + longer uptime on this instance).
- **Live growth measured directly, this session:** two `HandleCount`
  snapshots 30 seconds apart on PID 30088: 517,363 → 517,377 (**+14 in 31s,
  ≈ 27/min, ≈ 39,000/day** at that instant). This is the same order of
  magnitude as 08-08's lifetime-average rate (0.87/sec ≈ 75,000/day) and its
  live-sampled rate (up to 2.4/sec) — consistent with the same ongoing bug,
  not a new/different one.
- Event-log check (`Get-WinEvent`, Application log) found only older,
  unrelated crashes (`agentmux-cef.exe`/`libcef.dll` faults from July, an
  `agentmux-0.46.6` hang from June) — no direct corroboration there, but
  nothing contradicting this either; these predate the version in question.
- `muxlog srv grep mem_attribution` — the telemetry the 07-24/08-08 docs
  proposed building to make exactly this kind of gap self-diagnosing —
  returned **no matching lines** on this machine/version. Worth checking
  whether that instrumentation (referenced in `CLAUDE.md`'s muxlog table,
  commit `4b809d477`/PR #2483 per the 08-08 doc's own notes) actually shipped
  and is emitting, or whether it's present but silent under current
  conditions, or whether it never fully landed. Not chased further this
  session — flagged as the first thing to check before assuming new
  telemetry needs to be built from scratch.

## 4. Root cause: CONFIRMED — see §7

(Superseded — this section originally said "not pinned." The investigation
continued the same day; §7 has the full evidence chain. Left here only so
the doc's own history is legible: the leading hypothesis below turned out to
be correct, just needed one more step — reading `sysinfo`'s actual vendored
source instead of stopping at "it keeps file descriptors open for
performance.")

Per the 08-08 doc, `memmap2` and `portable-pty` were ruled out by source
inspection (neither calls `CreateFileMapping`/`Section` APIs on Windows).
The leading hypothesis was the `sysinfo` crate (v0.34.2)'s whole-machine
process-refresh calls in `agentmux-srv/src/backend/sysinfo.rs` — correct,
confirmed below.

## 5. Recommended next steps (superseded by §8 — kept for the record)

~~1. Check whether `mem_attribution` telemetry is actually live~~ — still
worth doing, unrelated to the fix.
~~2. Pin the exact leaking call via controlled bisection~~ — **not needed in
the end; reading the dependency's source directly was faster and more
precise than a bisection experiment would have been.**
3. A graceful srv-only restart path still does not exist — still relevant,
see §8.
4. Playbook note — still valid, kept for future recurrences of a *different*
mechanism.

## 6. Tools used (for reproducibility)

- `Get-CimInstance Win32_OperatingSystem` / `Win32_PageFileUsage` — system
  commit/page-file totals.
- `Get-Process` + `GetGuiResources` (P/Invoke via `Add-Type`) — per-process
  handle/GDI/USER counts across all `agentmux*` processes at once.
- Sysinternals `handle64.exe -accepteula -s -p <pid>` — handle-type
  breakdown by object type (already present at
  `%LOCALAPPDATA%\Temp\handle64.exe` on this machine from a prior session;
  not vendored in the repo).
- `node ~/.agentmux/shell/muxlog.mjs mem` and `... srv cat --grep <re>` —
  AgentMux's own built-in memory doctor and log search.

## 7. Root cause — confirmed, source-level (same day, follow-up)

**The bug: `sysinfo` v0.34.2's Windows backend leaks one kernel `Section`
handle on every single call to `refresh_processes_specifics`.** No live
tracing, bisection, or debugger attach was needed — reading the actual
vendored dependency source directly gave a complete, unambiguous answer.

**The exact code** (found locally at
`~/.cargo/registry/src/index.crates.io-.../sysinfo-0.34.2/src/windows/system.rs`,
lines 232–301 — this is the literal source Cargo compiles into
`agentmux-srv` per the pinned `sysinfo = "0.34"` in `agentmux-srv/Cargo.toml`):

```rust
let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
    Ok(handle) => handle,
    Err(_err) => {
        sysinfo_debug!(/* ... */);
        return 0;                    // ← leaks: no cleanup on this path
    }
};
// ... Process32FirstW/Process32NextW loop using `snapshot` ...
num_procs                            // ← leaks: no cleanup on this path either
```

**`snapshot` is a plain `windows::Win32::Foundation::HANDLE`** (imported
directly, confirmed via the file's `use` block) — a non-owning, `Copy`,
`repr(transparent)` newtype with no `Drop` implementation. `CloseHandle` is
never called on it anywhere in the file or the rest of the crate (confirmed
by grepping every `CloseHandle` call site in the vendored source — the only
two hits are for an unrelated `event` handle in `cpu.rs` and a `HandleWrapper`
type in `utils.rs` that the crate authors clearly know to use elsewhere, just
not here). Every single call — success or either error path — leaks the
handle `CreateToolhelp32Snapshot` returned.

**Why this produces exactly the observed symptom:**
- `CreateToolhelp32Snapshot`'s returned handle is Microsoft-documented to
  require `CloseHandle`
  ("[Snapshots of the System](https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/ToolHelp/snapshots-of-the-system.md)":
  *"If you do not destroy a snapshot, the process will leak memory until it
  exits."*) — the failure mode is a leak that only clears on process exit,
  matching every restart-reclaims-it observation in both this doc and 08-08.
- The handle type this API leaks is independently, publicly documented as
  `Section`-backed — this is the exact same fingerprint (near-100% anonymous
  `Section` handles) found on 2026-08-08 and reproduced live today (§3).
- **An independent, previously-published confirmation of the identical bug
  pattern exists**: [RUSTSEC-2025-0125](https://osv.dev/vulnerability/RUSTSEC-2025-0125),
  filed against the unrelated `thread-amount` crate, describes the *exact*
  same mistake — calling `CreateToolhelp32Snapshot` without a matching
  `CloseHandle` — with the advisory's own words: *"Repeated calls to this
  function will cause the handle count of the process to grow indefinitely,
  eventually leading to system instability or process termination when the
  handle limit is reached."* This is not a novel or exotic bug class; it's a
  known Rust/Windows footgun that has bitten more than one crate.
- **Call-frequency arithmetic matches the observed leak rate.** In
  `agentmux-srv/src/backend/sysinfo.rs`'s `run_sysinfo_loop` (the loop that's
  been running continuously since srv started), `sys.refresh_processes_specifics(...)`
  is called via **two separate call sites every single main-loop tick**
  (default 1s, configurable 0.2–2s via `telemetry:interval`): once for the
  light `ProcessesToUpdate::All` pass (line ~984, populates parent links) and
  once for the targeted `ProcessesToUpdate::Some(&all_pids)` pass (line
  ~1014, per-pane CPU/mem). **Both funnel through the same buggy function** —
  Windows' toolhelp API has no "snapshot just these PIDs" mode, so even the
  "targeted" call still does a full `CreateToolhelp32Snapshot` internally and
  filters client-side. A third call site fires every 30s in
  `log_memory_attribution` (the attribution feature literally built to
  diagnose this exact leak class — see the doc comment at
  `sysinfo.rs:126-138`, itself now retroactively describing its own root
  cause). A fourth, lower-frequency call site exists in
  `backend/reactive/registry.rs`'s `pid_alive()` helper (on-demand, not a
  fixed loop). At the default 1s interval: **~2 leaked handles/sec from the
  main loop alone ≈ 172,800/day** — the right order of magnitude against
  §3's live-measured ~27/min (≈39,000/day at that instant) and the 08-08
  doc's lifetime-average 0.87/sec (≈75,000/day); exact rate varies with
  configured interval, pane count (more panes → more PIDs in the targeted
  pass, but the snapshot cost is dominated by the fixed per-call leak, not
  pane count), and how long the urgent-attribution path (10s cooldown) fires
  under pressure.

**Confirmed fixed upstream, and the exact version range pinned:**
- `sysinfo` v0.34.2 (our pinned version) — **broken**, confirmed by reading
  the actual vendored source (above).
- `sysinfo` v0.35.0 — **fixed**, confirmed by fetching that tag's
  `src/windows/system.rs` directly: the handle is wrapped
  (`let snapshot = unsafe { Owned::new(snapshot) };` with the comment *"This
  owns the above handle and makes sure that close will be called when
  dropped"*) — a `windows`-crate RAII wrapper that closes on every path,
  including both early returns, automatically.
- `sysinfo` v0.36.0 and current `main` (latest release 0.39.6 at time of
  writing) — also fixed, same `Owned` wrapper pattern, confirmed by fetching
  both directly.
- The changelog's one Windows-leak entry ("Windows: Fix resource leak",
  v0.27.7) is a different, much older fix — already included in our 0.34.2,
  unrelated to this bug. No changelog entry explicitly documents *this* fix
  by name; it was found by direct source diffing, not changelog reading.

## 8. Fix applied — 2026-08-19, same day

**Bumped `sysinfo` from `"0.34"` to `"0.35"`** in `agentmux-srv/Cargo.toml`
(→ `0.35.2` in `Cargo.lock`) — deliberately the minimal fixed version, not
current stable `0.39.6`: **`sysinfo` 0.39.x requires Rust 1.95**, ahead of
this machine's toolchain (`rustc 1.93.0`); CI's `dtolnay/rust-toolchain@stable`
would likely satisfy that, but `0.35.2` gets the exact fix with zero MSRV
change and a far smaller `Cargo.lock` diff (avoids an unrelated `windows`
crate 0.57→0.62 major-version churn that came bundled with jumping straight
to 0.39.6). Revisit the `0.39` upgrade separately, unbundled from this fix,
once the toolchain question is resolved.

**Verification performed, all before opening the PR:**
1. `cargo build -p agentmux-srv` (release profile) — clean build, only
   pre-existing dead-code warnings unrelated to this change.
2. `cargo test -p agentmux-srv --bin agentmux-srv backend::sysinfo::` — all
   15 existing tests in the module pass unchanged, including
   `log_memory_attribution_runs_against_the_real_process_table_without_panicking`
   (exercises the exact fixed code path against the real Win32 process
   table).
3. **Added a new regression test**,
   `refresh_processes_specifics_does_not_leak_a_handle_per_call` — calls
   `sys.refresh_processes_specifics(ProcessesToUpdate::All, ...)` 500 times
   (the same call `run_sysinfo_loop` makes every tick) and asserts this
   process's own `GetProcessHandleCount` doesn't grow linearly with call
   count.
4. **Proved the test is a real discriminator, not just a green checkmark**:
   temporarily re-pinned to the broken `sysinfo = "0.34.2"`, rebuilt, and
   reran the new test — **it failed**, reporting *"handle count grew by 506
   over 500 refresh_processes_specifics calls"* — essentially exactly 1
   handle leaked per call, an exact match to the theorized mechanism in §7.
   Restored the `0.35` pin and reran — passes clean. This is the closing
   data point neither this doc's earlier draft nor the 08-08 doc were able
   to collect (no restart/rebuild had been performed against the live
   instance under investigation) — a controlled, repeatable, broken-vs-fixed
   A/B on the exact same code, not just an upstream changelog claim.

This also closes out `docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md`
— same bug, same fix, now landing via PR (see that doc for the original
live-incident details this fix resolves).

## 9. Multi-hour live soak test — completed, CONFIRMS THE FIX

Run against a fresh isolated portable build (`agentmux-0.55.16+g376006730`,
built directly from the merged fix commit `3760067`), targeting the real
`agentmux-srv` process (not its `--crash-monitor` child) running the actual
production `run_sysinfo_loop`. Sampled `handle64 -s -p <pid>`'s `Section`
count every 15 minutes for 4 hours (17 samples, 2026-08-19 12:39–16:39),
via a detached PowerShell monitor confirmed to survive independently of the
launching process (verified: still alive after its own parent process had
already exited).

| Time | Section handles | Total handles | Private MB |
|---|---|---|---|
| 12:39 (baseline) | 11 | 816 | 26.1 |
| 12:54 | 12 | 903 | 44.0 |
| 13:09 | 0† | 958 | 48.2 |
| 13:24 | 11 | 1,021 | 49.0 |
| 13:39–16:39 (13 more samples) | **11, every single sample** | 1,021 → 3,091 | 55.8 → 122.3 |

†One transient `0` reading at the 30-minute mark, immediately followed by
`11` again and staying there for the remaining 3h15m (14 consecutive
samples) — almost certainly a `handle64` read race (e.g. sampled mid- a
brief refresh window), not a real drop to zero live handles; not treated as
meaningful given every surrounding sample is consistent.

**Section handles stayed flat at 11 for the entire run.** Total handle
count and private memory both grew over the 4 hours (816→3,091 handles,
26→122 MB) — expected, ordinary churn in other handle types (File/Thread/
Process, consistent with normal idle-app background activity), and
specifically **not** the mechanism this fix addresses. The target process
was still alive and responsive at the end of the run (no crash, no restart
needed) — consistent with the fix not introducing any new instability.

This is the closing data point flagged as outstanding above: confirmed in
the actual `run_sysinfo_loop` production code path, over real multi-hour
uptime, not just the 500-call synthetic proxy. **Status: RESOLVED.**
