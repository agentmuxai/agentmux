# Status: `agentmux-srv` Leaking Windows Section Handles — Live Finding (2026-08-08)

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
  This is the *identical fingerprint* to the Audiosrv bug (anonymous
  pagefile-backed Section objects, never closed) — but this time genuinely
  inside AgentMux's own process, not an unrelated Windows service.
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
| `sysinfo` 0.34.2 | **Leading hypothesis, not proven** | Powers `agentmux-srv`'s 30s memory-attribution loop (`agentmux-srv/src/backend/sysinfo.rs:148`, `sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_memory())` — refreshes *every* process on the machine, every cycle). Its own docs disclose it "keeps a number of file descriptors open... for better performance when refreshing processes" — a real, by-design caching behavior. Zero literal `Section`/`CreateFileMapping` references found in its Windows backend source (`sysinfo-0.34.2/src/windows/*.rs`) though, so I could not point to one exact API call/line. `sysinfo-0.34.2/src/windows/process.rs:242` calls `GetModuleFileNameExW` once per newly-discovered PID (`ProcessInner::new`) — a very widely-used, not-typically-leaky API on its own, but the one Section-adjacent(ish) call found. |

**Arithmetic check that supports (but doesn't prove) the sysinfo/process-churn
theory**: 69.1h uptime ÷ 30s attribution cycle ≈ 8,292 cycles.
214,635 handles ÷ 8,292 cycles ≈ **~26 handles leaked per cycle** — a very
plausible number if a subset of ~26 processes per cycle (protected/SYSTEM
processes agentmux-srv can enumerate but not fully query, or newly-spawned
short-lived dev-tool processes like `git`/`cargo`/`npm`/`node`/`tsc`, which
this machine has constantly, including from this very agent's own work) each
leak one handle on a partial-permission or error path. **Not confirmed at
the code level** — this is a plausible-fit hypothesis from the numbers, not
a proven mechanism.

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
4. **Bisect the sysinfo hypothesis**, if pursued further: temporarily widen
   the `mem_attribution` refresh interval or narrow `ProcessesToUpdate::All`
   to a smaller process set on a test build, and see if the Section-handle
   growth rate changes proportionally — the same "suspend and watch the
   rate" bisection technique the 07-24 doc used for Traktor/Audiosrv (§16.2
   there).
