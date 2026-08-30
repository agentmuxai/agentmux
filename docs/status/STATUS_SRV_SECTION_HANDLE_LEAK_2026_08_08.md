# Status: `agentmux-srv` Leaking Windows Section Handles — Live Finding (2026-08-08)

> **RESOLVED 2026-08-29 (docs-cleanup Phase 3).** Root cause was the
> `sysinfo` crate's `CreateToolhelp32Snapshot` handle leak. Fixed by
> **#2666** (`fix(srv): bump sysinfo 0.34→0.35 to fix
> CreateToolhelp32Snapshot handle leak`) and confirmed by a 4-hour soak
> test recorded in **#2673**. The follow-up recurrence investigation
> (`STATUS_SRV_SECTION_HANDLE_LEAK_LIVE_RECURRENCE_2026_08_19.md`) is
> resolved by the same fix.
>
> A *second, separate* handle leak in the same process — `FsWatchPool`'s
> health sweep leaking a File+Semaphore pair per tick — was found later and
> fixed independently in **#2722**; see
> `STATUS_FS_WATCH_SWEEP_HANDLE_LEAK_2026_08_22.md`. Do not conflate the
> two: different mechanisms, different fixes, both now shipped.
>
> **Caveat that outlived the fix:** an `agentmux-srv` process started
> before #2666 shipped still carries the leak until it is restarted — the
> fix is in the binary, not applied to already-running instances. A live
> sighting on an old process is not a regression.
>
> Everything below is preserved as the original investigation record.

> **Update 2026-08-09, ~13:22 UTC — the leaked process died and was
> auto-recovered before the manual restart in §5 was ever executed.** Root
> cause is still NOT fixed in code; this is a live data point on the
> self-healing path, not a resolution. See §17.
> *(Superseded by the RESOLVED banner above — accurate when written.)*

**Related, but a different mechanism:** `docs/status/STATUS_PF_COMMIT_GROWTH_INVESTIGATION_2026_07_24.md`
(RESOLVED 2026-07-25 — that investigation's root cause was an **Audiosrv**
Section-handle leak, unrelated to AgentMux, fixed by `Restart-Service
Audiosrv`). This doc is a **new, separate finding**: this time the leak is
inside `agentmux-srv` itself. Read that doc first for the methodology this
one reuses (handle-type summary via Sysinternals `handle.exe`, primary
counters over derived proxies, rate/uptime correlation) and for why
"unattributed commit" investigations on this class of problem need to check
multiple independent sources, not assume the same culprit twice.

**Status: root cause narrowed to `agentmux-srv`'s own process; exact
triggering API call not yet pinned. Live mitigation (restart srv) identified
but NOT executed pending explicit user confirmation** — see §5, this is a
much higher-blast-radius action than the Audiosrv fix was.

## 1. Trigger

User reported unbounded pagefile growth with **very little pane activity
this session (~4 pane opens)** — inconsistent with both (a) the renderer-count
GPU-driver-commit mechanism (`#2218`, needs many panes/windows to matter at
~1GB each) and (b) the original Audiosrv leak (already fixed, and this
machine's current Audiosrv handle count is normal — see §2). User's own prior
experience diagnosing a similar issue on a different Windows 10 machine
pointed at Audiosrv-style service leaks and "metering" as the right approach
— prompting a live re-run of that same methodology on this machine.

## 2. What was ruled out

- **Audiosrv**: 691 handles at check time — nowhere near the ~85K that
  indicated the original bug. Not currently a contributor on this machine.
- **GPU/driver-pooled commit (`#2218`'s mechanism)**: not investigated this
  session (a distinct, already-shipped-and-reasoned-about mechanism per the
  07-24 doc) — the numbers below don't need it to explain what's happening.

## 3. Live findings, this machine, this session

- **System commit: 133.12 GB / 140.37 GB commit limit — 95% full** at time
  of check. This is acute, not a slow background trend — closer to
  exhaustion than the 07-24 investigation's worst point (79.5 GB before that
  fix).
- Top-handle-count process on the machine by a wide margin:
  **`agentmux-srv-0.54.10-windows.x64.exe`, PID 45728 — 215,705 handles**
  (next highest: `node` at 24,797, `chrome` at 11,073). This is the
  **shared-channel instance** (`local-main-b28b7a-92c05375`), not a `dev:`
  build — it backs multiple concurrent panes, not just one (see §5).
- `handle64 -s -p 45728` (Sysinternals Handle, downloaded from
  `live.sysinternals.com` for this check — not currently vendored/kept in
  the repo): **214,636 of 215,597 handles (99.5%) are `Section` objects.**
  Everything else (Event, File, Process, Thread, etc.) is small and
  unremarkable — 357 Process handles, 146 Thread handles, etc.
- `handle64 -a -p 45728` (full listing): the Section handles are
  **overwhelmingly unnamed/anonymous** — one legitimate named exception
  (`\Sessions\1\BaseNamedObjects\windows_shell_global_counters`, a normal
  per-process object every process has), the rest have no name field at all.
  This *resembles* the Audiosrv bug's fingerprint (anonymous Section
  objects, never closed) — but this time inside AgentMux's own process, not
  an unrelated Windows service. **Caveat (codex P1 on the PR for this
  doc): an anonymous Section handle does not by itself prove the section
  is pagefile-backed or how much commit it holds** — the Audiosrv case
  verified the commit link with a decisive before/after restart
  measurement (16.4 GB reclaimed in ~9s, §16.3 of the 07-24 doc); nothing
  equivalent was captured here at write-up time, so "these handles ARE the
  commit growth" was a hypothesis, not established. §17.2's later data is
  *consistent with* the link (commit dropped 133→76.65 GB across the
  process's death) but is not a clean single-variable measurement (the
  commit limit moved too, and other processes churned in the same window).
  The clean verification, if this recurs: snapshot commit, kill/restart
  just that PID, snapshot again within a minute — or VMMap the process and
  read the actual committed size of its Section objects before acting.
- **Process uptime**: started 2026-08-05 02:49, ~69h old at check time.
  215,705 handles ÷ ~248,783s uptime ≈ **0.87 handles/sec average** over the
  process's whole life.
- **Live sampling** (18 samples, 5s apart, 90s total): net growth ~212
  handles in 90s ≈ **~2.4 handles/sec right now** — faster than the
  lifetime average, with occasional small dips (some handles ARE being
  closed, just outpaced by creation), consistent with growth rate scaling
  with machine activity/process churn rather than a fixed timer.

## 4. Root-cause narrowing (not fully pinned)

Checked the three most plausible dependency-level candidates directly
against source (`Cargo.lock` versions, then the actual crate source under
`~/.cargo/registry/src/`):

| Candidate | Verdict | Evidence |
|---|---|---|
| `memmap2` (0.8.0 / 0.9.10, both in the tree) | **Doesn't fit** | Only pulled in via `minidump-writer` (crash-dump generation — fires on an actual crash, not continuously) and `smithay-client-toolkit`/`xkbcommon` (Linux/Wayland-only, irrelevant on Windows). |
| `portable-pty` 0.9.0 (the CLI subprocess PTY layer) | **Ruled out** | Zero `CreateFileMapping`/`MapViewOfFile`/`Section` references anywhere in its source. |
| `sysinfo` 0.34.2 | **Leading hypothesis, not proven** | Powers `agentmux-srv`'s whole-machine process refreshes. **Two independent call paths, not one (codex P1 on the PR for this doc — the original version of this row named only the first):** (a) the 30s memory-attribution snapshot (`agentmux-srv/src/backend/sysinfo.rs:148`, `refresh_processes_specifics(ProcessesToUpdate::All, ..., nothing().with_memory())`), and (b) **the main telemetry tick** — `run_sysinfo_loop` also calls `refresh_processes_specifics(ProcessesToUpdate::All, ...)` every 0.2–2s whenever `pidregistry` is nonempty (`sysinfo.rs:617-621`, plus a per-pane deep refresh at `:647-651`). The per-tick path runs ~15–150× more often than the attribution path, so any per-refresh leak indictment applies to it first. sysinfo's own docs disclose it "keeps a number of file descriptors open... for better performance when refreshing processes" — real, by-design caching. Zero literal `Section`/`CreateFileMapping` references in its Windows backend source, so no exact API call/line was pinned; `GetModuleFileNameExW` per newly-discovered PID (`process.rs:242`) is the one Section-adjacent(ish) call found. |

**Arithmetic check — corrected (codex P1):** the original version of this
section divided 214,635 handles by the 30s attribution cadence alone
(≈8,292 cycles → "~26 handles/cycle") to argue plausibility. That
arithmetic wrongly singled out the attribution loop: with the per-tick
whole-machine refresh (path (b) above) also running — at 1s default
interval, ~248,000 ticks over the same 69h — the same total is equally
consistent with **<1 handle per tick** on the far more frequent path.
The observed ~0.87/sec average is in fact *closer* to "roughly one per
1s telemetry tick" than to "~26 per 30s cycle." Both remain unproven;
the actionable consequence is that any bisection experiment must
instrument or disable **both** paths (or stagger their intervals so a
30s-vs-1s growth staircase would distinguish them) — widening only
`ATTRIBUTION_INTERVAL` and seeing no change would NOT clear `sysinfo`.

**No prior documentation of this exact issue was found** — grepped
`agentmux-srv/src`, `docs/retro/`, `docs/specs/` for "section handle",
"handle leak", "leaked handle" and found nothing. This is new.

## 5. Live mitigation — identified, NOT executed

Restarting `agentmux-srv` (by PID, per `CLAUDE.md`'s "always kill by PID,
never by image name" rule) should reclaim the leaked handles/commit the same
way `Restart-Service Audiosrv` did for the original bug — srv death is a
first-class supervised event (`agentmux-launcher`'s exit-triggered respawn,
`docs/specs/SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11.md`).

**This is NOT a free/silent fix like the Audiosrv restart was**, confirmed
by tracing the actual supervision path before considering doing this live:

- Killing srv **also deliberately recycles the entire CEF host** (all
  windows) — by design (`SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11.md`: "the
  host is disposable now... killing and respawning it costs seconds and
  self-restores the window set"). Every open window flashes through a
  "Restoring session…" splash and reprojects from srv's durable state.
  Window position/size is approximate after reproject, not pixel-exact.
- **Every live PTY-backed CLI subprocess (`claude.exe` etc.) srv was
  tracking is hard-killed**, not orphaned-and-reconnected — each is
  assigned to a per-block Windows Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, owned inside srv's own process
  (`agentmux-srv/src/backend/process_tracker/windows.rs`). When srv's
  process handles close, the kernel kills every tracked child instantly.
  Conversation history is **not** lost (durable SQLite/WAL — session id
  persists), but the in-flight turn is cut off: the pane shows an
  "interrupted" banner, and the user's *next* message triggers a fresh CLI
  process via `--resume <session_id>` to reattach — the turn itself is not
  seamlessly continued.
- **PID 45728 is the shared-channel instance backing multiple concurrent
  panes**, confirmed via `muxspect list` at the moment of this check: at
  least 4 persistent agent panes (including the one this very investigation
  was run from) and one shell pane were live under this srv. Restarting it
  affects all of them at once, not just one pane — including, very likely,
  cutting off whichever conversation is used to action this fix.

**Recommendation:** get explicit confirmation from a human before restarting,
ideally when as few panes as possible are mid-turn. This is the reason this
doc exists as a separate handoff rather than the fix just being applied
directly — the blast radius genuinely needed to be traced first, unlike the
Audiosrv case where `Restart-Service` only affected one unrelated OS service.

## 6. Suggested next steps

1. **If/when approved**: restart `agentmux-srv` PID (by PID, not image
   name), confirm handle count and commit charge both drop afterward the
   same way §16.3 of the 07-24 doc confirmed the Audiosrv fix (before/after
   table), and record the actual numbers here or in a follow-up doc.
2. **Pin the exact trigger**, if this recurs: periodic `handle -s -p <pid>`
   sampling (the binary is at `%TEMP%\handle64.exe` on this machine from
   this session, not vendored in the repo) correlated against
   `agentmux-srv`'s own `mem_attribution` log timestamps (`muxlog srv grep
   mem_attribution` — note this returned no output on this machine's current
   log file at check time; worth confirming the log line/level is actually
   active before relying on it for a future correlation pass). The 07-24
   doc's §16.5 "timer signature" lesson applies directly: if the growth rate
   turns out to be constant regardless of workload (not yet checked here,
   only a 90s live sample was taken), that's evidence for a
   timer/interval-driven cause over an activity-driven one.
3. **Product idea, same one the 07-24 doc already flagged and still
   unbuilt**: `mem_attribution` (`agentmux-srv/src/backend/sysinfo.rs`)
   only buckets process-*private* commit today, so this entire class of
   leak (kernel objects charged to no process, or — as found here — charged
   to AgentMux's own process but as handle count rather than private bytes)
   is invisible to it. A handle-count-anomaly heuristic (alert if any
   AgentMux process's own handle count exceeds some threshold, e.g. 10K)
   would have caught this automatically instead of requiring a manual
   Sysinternals pass.
4. **Bisect the sysinfo hypothesis**, if pursued further — covering BOTH
   whole-machine refresh paths (§4's correction), not just the attribution
   loop: stagger the two intervals (e.g. attribution at 30s, telemetry tick
   at 1s) and look for a matching growth staircase, or disable each path in
   turn on a test build and watch the Section-handle rate — the same
   "suspend and watch the rate" bisection technique the 07-24 doc used for
   Traktor/Audiosrv (§16.2 there). Widening only `ATTRIBUTION_INTERVAL`
   and seeing no change would NOT clear `sysinfo`, since the per-tick path
   would still be running.

## 17. Update 2026-08-09 — PID 45728 went down on its own; auto-recovery worked exactly as designed

Between the original write-up (§1-16, 2026-08-08) and this update, nobody
executed the manual restart §5 held off on. The leaked process died by
itself and the launcher's existing supervised-recovery path (traced in this
same conversation, separately, when checking whether that path was safe to
trigger deliberately) handled it automatically. **This section is a live
data point, not confirmation of a code fix** — the root cause from §4
remains unpatched, so the same trajectory can recur on the replacement
process.

### 17.1 What happened, from the launcher log directly

`agentmux-launcher.log` (channel `local-main-b28b7a-92c05375`) for PID 45728:

- **2026-08-09T11:57:57Z** — the last live log lines from PID 45728: a burst
  of `agent health transition ... old=Idle new=Exited` and
  `session_recovery: failed to clear active pid ... error=not found` across
  every block it was tracking (including this conversation's own block at
  the time, `97c97310-...`). No panic message, no explicit error, no crash
  dump written to `C:\CrashDumps\agentmuxsrv\` for this PID — the process
  simply stopped logging. Given §3's live reading shortly before this
  (system commit at 133.12 GB / 140.37 GB, 95%, with PID 45728 itself
  holding 215,705 handles), an externally-forced termination (OOM-adjacent
  kill, allocation failure) fits the evidence better than a clean
  self-detected crash — but this is inference from absence of a dump/panic,
  not a confirmed mechanism.
- **2026-08-09T12:04:03Z** (~6 minutes later) — a fresh `agentmux-srv`
  (PID 62888, same version 0.54.10) logs `agentmuxsrv starting`. This is
  the launcher's exit-triggered respawn (`agentmux-launcher/src/supervisor/`,
  traced in the same conversation as this update — see the "is it safe to
  restart" investigation for the mechanism this exercised for real).
- **2026-08-09T13:22:27Z** (host recycle + reproject landed later, once
  this pane's tab resumed activity) — this conversation's own persistent
  CLI process respawned with `--resume 556ddd36-dcdd-4798-b8b7-70ad8c233632`
  — its own real session id, matching this very conversation. **First-hand
  confirmation the documented recovery path
  (`docs/specs/SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11.md`) works as
  designed**: this conversation was not visibly interrupted (the crash
  landed between turns, not mid-turn, so there was nothing in flight to
  lose), and resumed on the new process with full history intact via the
  CLI's own `--resume`, not a fresh conversation.
- Block ids changed across the recycle (this pane's own block id is now
  `93000821-...`, not the original `97c97310-...`) — consistent with
  `SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md`'s "transient/in-flight
  state is dropped on reproject, re-derived from durable topology instead
  of preserved 1:1" model already documented for this recovery path.

### 17.2 Commit charge: partially reclaimed, not fully

| | Before (§3, 2026-08-08) | After (2026-08-09, ~13:24 UTC) |
|---|---|---|
| System commit | 133.12 GB / 140.37 GB (95%) | 76.65 GB / 86.45 GB (**88.7%**) |
| Leaked PID | 45728, 215,705 handles (214,636 Section) | gone |

Note the **commit limit itself also dropped** (140.37 → 86.45 GB) — this
machine's pagefile is system-managed (per the original
`STATUS_PF_COMMIT_GROWTH_INVESTIGATION_2026_07_24.md`'s own findings on
this same box), so the ceiling floats with recent demand/disk headroom, not
just the numerator. System uptime is unrelated (`LastBootUpTime` confirmed
unchanged, ~13 days) — no reboot happened, only the one process restarted.

88.7% is still elevated, not a clean baseline — some of that is the
now-familiar mix of concurrent AgentMux dev/portable instances (§3's
process list) plus whatever else is running on this shared dev machine, not
attributable to this specific leak. Not investigated further this session.

### 17.3 The replacement process, checked immediately

PID 62888, ~1h22min old at check time:

```
Total handles: 942
Section: 172
```

Low/normal baseline — the leak has **not** measurably recurred yet on the
fresh process. Consistent with §16.4's arithmetic (the original PID took
~69 hours to accumulate to a problematic level) — too early to conclude
anything from this alone, just recorded here as the starting point for
whoever next checks this process's handle count.

### 17.4 What this update does and doesn't change

- **Does not fix the root cause.** §4's leading hypothesis (`sysinfo`'s 30s
  whole-machine process refresh, `agentmux-srv/src/backend/sysinfo.rs:148`)
  is unpatched. Nothing in this update touched code.
- **Does confirm the supervised-recovery path is real and works**, for
  whoever is deciding whether a manual `taskkill /PID <srv> /F` is safe to
  use as a stopgap the next time commit climbs dangerously high: yes, per
  this live, unplanned exercise of the exact same path — expect a full
  host recycle (all windows reproject, brief visible interruption) and any
  in-flight turn to be cut and resumed via `--resume` on the next message,
  not preserved mid-turn.
- **Does not mean this can be left alone.** The same trajectory — slow
  Section-handle growth, unclear exact trigger, eventual forced
  termination — will very likely repeat on PID 62888 (or whatever succeeds
  it) on a similar timescale, until §4's actual leak source is found and
  patched. This status remains open.
