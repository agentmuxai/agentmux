# Status: Windows Pagefile/Commit-Charge Growth During Live Use — Investigation Handoff (2026-07-24)

> **RESOLVED 2026-07-25 — see §16.** Root cause found and live-proven: the Windows Audio
> service (Audiosrv) on this machine leaks ~1 pagefile-backed shared-memory Section (~200 KB)
> per second from boot, ~17 GB/day of commit attributed to no process. `Restart-Service
> Audiosrv` (elevated) reclaimed 16.4 GB instantly. **AgentMux is exonerated.** §13-§15's
> GPU-pooling conclusions are superseded — they were artifacts of measuring against this
> constant background leak. Everything below is preserved as the investigation record.

Written mid-session at the user's request, right before a machine restart, so the next agent
(possibly a future instance of this same agent) doesn't have to re-derive context. The user is
restarting specifically to reset system commit charge from **69 GB down to ~8 GB**, then wants
rigorous telemetry put in place during an ordinary work session (not a synthetic test) to find
exactly what's driving the growth back up.

## TL;DR

- **This is a live, unresolved investigation, not a fixed bug.** Nothing has shipped from this
  session. Don't confuse it with issue **#2218** (renderer-pool/GPU-driver commit growth from
  *opening new panes/windows*) — that one's three fix PRs (#2220, #2221, #2222) are already
  merged and confirmed shipped in `main`/`v0.54.4`. This is a **different, newer, still-open**
  symptom: PF growth during **normal interactive use of an already-open agent pane**, reported
  directly by the user watching Task Manager live.
- **Key user-supplied clue, not yet acted on:** the user says **macOS does not show this
  problem**. Whatever the mechanism, it is either Windows-specific in cause, or Windows-specific
  in how visibly it's reported (commit-charge accounting vs. macOS's RSS/compression reporting).
  Any theory that doesn't explain the platform asymmetry is incomplete.
- **What this session confirmed live:** the CLI process backing a pane (`claude.exe`) is
  long-lived and reused across every message — `agentmux-srv`'s `PersistentSubprocessController`
  never respawns it per-message (confirmed via `persistent.rs:334-337`). Sending a message does
  **not** spawn a new OS process. So "which process grows" narrows to that one persistent
  process, not a fresh one per turn.
- **What a short controlled test did NOT find:** 6 short, tool-free synthetic messages sent
  ~90s apart over ~10 minutes showed transient per-message spikes (`claude.exe` swinging
  ~372→~700 MB, system commit swinging in step by ~500-900 MB) that **fully resolved** back to
  a stable floor within the settle window every time, with no clear monotonic ratchet in the
  floor itself (~71,241 MB → ~71,325 MB net drift over the whole run — noise-level against a
  71 GB total). See §3 for the full data and, importantly, §5 for why this test almost
  certainly wasn't representative of what's actually happening in the user's real 69 GB session.
- **Known measurement flaw in this session's own tooling** — the PowerShell sampler used
  `WorkingSetSize` (resident set) as a per-process memory proxy. `retro-commit-charge-pagefile-growth-2026-07-02.md`
  §5 already flagged this exact class of mistake for `agentmux-srv`'s own `sysinfo.rs`
  attribution code (`proc.virtual_memory()` there): working-set/RSS is **not** the same thing
  as commit charge. The next telemetry pass must use `Private Bytes` / `Pagefile Bytes`
  per-process counters, not working set, or it will repeat the same measurement error.

## 1. Why this is distinct from #2218

`#2218` ("renderer pool commit-charge growth") is about the cost of **opening additional
panes/windows** — each new CEF renderer causes the GPU driver to commit ~1 GB of invisible,
kernel-mode, pagefile-backed memory (charged to the System process, PID 4), per
`retro-commit-charge-pagefile-growth-2026-07-02.md` and `retro-commit-restart-reclaim-2026-07-16.md`.
Its three remaining fix items (B.4 deterministic browser teardown, B.5 pool eviction under
pressure, ratio-aware pressure thresholds) all merged 2026-07-18 (#2221, #2220, #2222). The
GitHub tracking issue **#2218 itself is still open** on GitHub despite all three shipping —
that looks like an oversight (nobody circled back to close it), separate from this
investigation. Worth closing once someone confirms it live, but it's not blocking this thread.

**This investigation is about a different mechanism entirely**: the user explicitly said
"it is not panes and windows, it is interaction with claude in the agent pane" — i.e. PF grows
during ordinary message-send/response-processing in a pane that's already open, no new
panes/windows involved. Keep these two investigations separate in any future write-up; don't
let fixes/findings for one get attributed to the other.

## 2. What's confirmed about the mechanism so far

- `PersistentSubprocessController::send_message` (`agentmux-srv/src/backend/blockcontroller/persistent.rs:334-337`)
  only calls `spawn_process` if `!self.is_running()` — every turn after the first reuses the same
  long-lived CLI stdin/stdout pipe. `send_user_message` (the steering path, `persistent.rs:406-435`)
  never spawns at all. **No new process per message** — ruled out as a mechanism, confirmed both
  by code read and by live process-list sampling during this session (same `claude.exe` PID
  throughout).
- Live process tree at investigation time: `agentmux-srv-0.54.4-windows.x64.exe` (PID 37664) is
  the parent of `claude.exe` (PID 7220 in this session) — matches the architecture read.
- A background research pass (see agent transcript from this session, "Research what happens on
  agent message send") ranked candidate mechanisms **before** any live test was run. Its top
  candidate — full markdown-tree reparse on every streamed chunk in
  `frontend/app/element/markdown.tsx:202-282` (a documented, previously root-caused *latency*
  issue per `docs/analysis/ANALYSIS_AGENT_PANE_TYPING_LATENCY_2026_05_30.md`, but never
  evaluated for memory impact) — was **not supported** by the live data: the renderer process
  hosting that code stayed flat (~582-600 MB) across every test message, including a 1800-word,
  6-code-block, 3-table synthetic response designed specifically to stress that code path.
  **This candidate should be considered deprioritized, not eliminated** — the live test used
  short, simple responses; markdown-heavy real coding output (large diffs, long file dumps in
  code fences) could still stress it differently.

## 3. Controlled test data (this session, pre-restart)

Method: `mem_sample.ps1` (background PowerShell sampler, 1 Hz) + `mem_markers.log` (manual
phase timestamps) in `C:\Users\area54\.agentmux\agents\agenta-07017\`. Raw CSVs from this
session are in that same directory (`mem_sample.csv`) if still present — **not committed to the
repo**, this was scratch/local investigation state only.

6 synthetic messages (short, tool-free, single-shot text responses via the `Agent` tool — a
proxy for "send a message to a Claude pane," not a real multi-turn session), ~90s apart,
`claude.exe` floor sampled after each ~75-90s settle window:

| After message | `claude.exe` floor (WorkingSet, MB) | System commit floor (MB) |
|---|---|---|
| 1 | 372.4 | 71,241 |
| 2 | 371.9 | 71,255 |
| 3 | 370.4 | 71,305 |
| 4 | 375.0 | 71,292 |
| 5 | 399.6 | 71,328 |
| 6 | 371.7 | 71,325 |

No monotonic ratchet — message 5's floor bump reverted by message 6. Net system-commit drift
over the whole ~10-minute run: ~85 MB, noise-level. Renderer (~582-600 MB), GPU process
(~126-129 MB), and `agentmux-srv` (~66-70 MB) were all flat throughout, showing no correlation
with message activity.

**Conclusion from this data alone: no leak found.** But see §5 — this test almost certainly
wasn't stressful enough to reproduce what the user is actually experiencing (69 GB commit in
real use).

## 4. The Windows-vs-macOS clue (unexplored — highest-value open thread)

The user reports macOS does not show this problem running the same app. Two non-exclusive
explanations, neither investigated yet:

1. **Accounting difference, not behavior difference**: Windows "commit charge" reflects virtual
   memory *reservation*, and Windows/V8's default allocator is known in general to be less
   eager about returning freed pages to the OS than macOS's allocator (which also transparently
   compresses/purges background memory, so Activity Monitor rarely shows the same growth even
   when heap behavior is identical). If true, the underlying `claude` CLI behavior could be
   platform-agnostic, and the "fix" would be entirely about Windows-side mitigation (periodic
   recycle, allocator tuning, or accepting it as an OS quirk) rather than an app bug.
2. **Genuine Windows-specific code path**: something in how `agentmux-srv` spawns/pipes the CLI
   on Windows specifically (PTY vs. pipe handling, console allocation — cf.
   `retro-login-console-visibility-not-pty-2026-07-23.md` for unrelated but architecturally
   similar Windows-only console/PTY quirks found earlier this same week) could differ from the
   macOS spawn path in a way that affects memory retention.

**This needs to be tested, not guessed at** — see plan below.

## 5. Why the controlled test (§3) likely undersells the real problem

The synthetic test was deliberately lightweight to keep it safe and fast on a machine already
showing a low-memory banner. It does **not** match the user's real usage pattern in several
ways that could each independently explain the gap between "no leak found" (§3) and "69 GB
commit in practice":

- **No tool calls.** Real sessions include file reads, greps, diffs, bash output — all of which
  get held in the growing conversation transcript that the persistent CLI process must keep in
  memory for the life of the session. The synthetic test explicitly avoided tools.
  Tool-call-heavy work should be part of the next test, not excluded.
- **No accumulating context.** Each synthetic message was a fresh, independent subagent
  call (no shared history growing turn-over-turn). A real session's context grows monotonically
  with every turn — that alone predicts *some* real, expected growth that a short/independent-message
  test structurally cannot see.
- **Single pane, ~10 minutes.** The user's real 69 GB session presumably involves multiple
  panes/instances over a much longer working period (hours, not minutes) — both dimensions
  (pane count, session duration) were minimized in this test for safety and speed.
- **Measurement granularity/metric may be wrong** — see the `WorkingSetSize` vs. commit-charge
  caveat in the TL;DR. If the true growth is in *committed-but-not-resident* pages (classic
  pagefile-backed reservation that doesn't show in working set), this test's own instrumentation
  would have been blind to it even if it was happening in real time.

## 6. Plan for the next agent (post-restart)

Goal: **rigorous, passive telemetry during a real, otherwise-unrelated work session**, so the
signal is real usage, not synthetic load — the user explicitly wants this to run in the
background while doing normal work, not as a dedicated stress test.

### 6.1 Immediately after restart, before any other work

1. Confirm system commit charge is back down near the expected ~8 GB floor (`Get-Counter
   '\Memory\Committed Bytes'`) — this is the clean baseline everything else compares against.
   Record it verbatim in whatever follow-up doc/log this produces.
2. Note exactly what's running at that baseline (process list snapshot) so "baseline noise" is
   characterized before load starts.

### 6.2 Fix the measurement methodology (don't repeat this session's mistakes)

1. **Switch from `WorkingSetSize` to true commit/private-bytes counters.** Use
   `Get-Counter '\Process(claude*)\Private Bytes'` and, ideally, `\Process(claude*)\Pool Nonpaged Bytes`
   / `Pagefile Bytes` (per-process pagefile usage is the direct analog of the system-wide
   "Committed Bytes" counter this session used at the system level) — Working Set is resident
   memory and can under- or over-state true commit contribution. This exact class of error was
   already called out once for `agentmux-srv`'s own attribution code
   (`retro-commit-charge-pagefile-growth-2026-07-02.md` §5) — don't reintroduce it in ad-hoc
   tooling.
2. **Sample less aggressively but for much longer.** 1 Hz was fine for a 10-minute synthetic
   test; for an hours-long passive background session, 1 sample per 5-10s is enough resolution
   and avoids `Get-CimInstance`/`Get-Counter` overhead accumulating its own noise over a long run.
3. **Correlate against real event timestamps from srv's own logs, not manual markers.**
   This session hand-typed `mem_markers.log` entries around synthetic `Agent` tool calls — that
   doesn't scale to a real work session and won't exist for the user's own real message sends.
   Instead, tail the live srv log for actual turn-boundary events (`muxlog srv grep` for
   whatever the send-message / turn-start / turn-end log lines are — check
   `agentmux-srv/src/backend/blockcontroller/persistent.rs` and `agent_session` logging for the
   exact line format) and join the memory samples against those timestamps after the fact. This
   makes the telemetry fully passive — no need to fake activity, it just observes whatever the
   user is actually doing.
4. **Track per-pane, not just per-process-name.** If multiple panes are open, `claude.exe`
   process names alone won't disambiguate which pane a given process's growth belongs to — use
   parent PID (`agentmux-srv` child) or, better, whatever pane/block ID the srv logs already
   attach to each subprocess, so growth can be attributed to a specific long-running pane rather
   than lumped into one aggregate number.

### 6.3 Run it

1. Start the passive sampler (fixed per §6.2) right after the clean-baseline snapshot, then just
   go do the "other misc stuff" work the user mentioned — real coding tasks, real tool calls,
   real multi-turn conversation, ideally in the same pane(s) for an extended period (aim for at
   least an hour, longer if feasible) so context genuinely accumulates and any slow-drift effect
   has time to show up above noise.
2. Periodically (e.g. every 15-20 min) snapshot the *floor* (post-idle-settle low point, not the
   transient peak) the same way this session did in §3, so there's a clean growth-over-time
   series independent of moment-to-moment GC noise.
3. If a real upward trend in the floor appears, narrow it down:
   - Does it correlate with tool-call volume/size specifically (vs. plain conversational
     turns)? — tests the "growing transcript" hypothesis from §5.
   - Does it correlate with pane count (open a second pane mid-session, see if the trend
     changes slope)? — tests whether this compounds with #2218's already-fixed mechanism or is
     independent.
   - If possible/safe, get comparative data from a macOS machine running the identical session
     shape — this is the single most direct way to resolve §4's open question, and should be
     treated as high priority if a macOS machine is available, since it's the one piece of
     evidence that could immediately rule in/out "Windows accounting artifact" vs. "real
     Windows-only leak."

### 6.4 Write up findings

Follow this doc's own convention (a dated status/retro doc in `docs/status/` or `docs/retro/`,
cross-linking back to this one and to `#2218`/`retro-commit-charge-pagefile-growth-2026-07-02.md`/
`retro-commit-restart-reclaim-2026-07-16.md`) so whoever picks this up next — human or agent —
has the same continuity this doc is trying to provide right now.

## 7. Environment notes for continuity

- Investigation happened on branch `agenta/claude-global-revocation-and-fixes` (this repo,
  `C:\Users\area54\.agentmux\agents\agenta-07017\agentmux`), against a portable build already
  compiled from that branch's HEAD (`d767690d`) — that build was running live during this
  session at `C:\Users\area54\Desktop\agentmux-0.54.4+gd767690d.20260724T054742.31553-x64-portable\`.
  It may or may not still exist/be running post-restart; rebuild if needed (`task package`),
  or use `task dev` for a live-reload loop if the telemetry work will involve touching
  `agentmux-srv`/`agentmux-cef` source rather than just external observation.
- This is unrelated to the `agenta/claude-global-revocation-and-fixes` branch's actual code
  changes (Claude Armory revocation-through-broker work) — that work is still pending a
  changeset + push + PR, tracked separately, not part of this investigation.
- Per standing instruction from this session: **operate only in this agent's own clone**
  (`agentmux/` inside this agent's own working directory) for any git work — do not touch
  sibling clones/worktrees found elsewhere under `~` (e.g. `amux-release`,
  `agentmux-wt-swarm-dupe-fix`) even if they look related.

## 8. Restart continuity log (2026-07-24, follow-up session)

- Machine restarted as planned. User observed system commit at **~4.6 GB** shortly after
  restart with AgentMux as effectively the only app running — better than the ~8 GB floor
  this doc's §6.1 anticipated. By the time this follow-up session queried it directly
  (`Get-Counter '\Memory\Committed Bytes'`), it had drifted to **5.72 GB** as this session's
  own AgentMux processes + one `claude.exe` pane spun up — expected, not a concern.
- **A visible "Agent encountered an error" occurred in-pane right at session start.**
  Root-caused via `muxlog srv grep`, not guessed: `[process-tracker] assign_process failed`
  followed by `stale --resume session id unreachable under the current config dir — clearing
  so the next message starts a fresh conversation` (srv log, ~11:03:52–53, recurred once more
  at 11:04:27; an identical pair also appears at 10:06:17–18, i.e. this is a repeatable
  post-restart pattern, not a one-off). Cause: the persistent CLI process's `--resume` session
  ID pointed at pre-restart state that no longer resolved. The controller self-healed (cleared
  the stale ID, next message started a fresh conversation) — the in-flight turn during that
  recovery is what surfaced as a user-visible error. Net effect: that pane's conversation
  history reset; nothing structurally broken. **Unrelated to the PF investigation itself** —
  noted here only because it happened during this session's restart and could otherwise look
  like a mystery to whoever picks this up next.
- **Telemetry v2 started**, fixing the §6.2 measurement-methodology gap: `pf_telemetry_v2.ps1`
  (in this agent's own dir, `C:\Users\area54\.agentmux\agents\agenta-07017\`) samples
  `\Memory\Committed Bytes` (system) plus `\Process(claude*)\Private Bytes` and
  `\Process(agentmux*)\Private Bytes` (true commit-relevant counters, not WorkingSet) every 7s,
  appending to `mem_sample_v2.csv` in the same directory. Launched as a **detached background
  process** (`Start-Process -WindowStyle Hidden`, PID recorded at launch time) so it survives
  independently of any single chat session — check `Get-Process -Id <pid>` or just look for
  fresh rows in the CSV to confirm it's still alive. Not yet joined against srv log turn-boundary
  timestamps (§6.2.3) — that join is still a TODO for whoever analyzes the resulting CSV.
- Next: let the user do normal work for an extended period (§6.3) with this sampler running
  passively in the background, then come back and look for a rising floor in the CSV.

## 9. Significant interim finding (2026-07-24, ~1hr into this session) — possible #2218 recurrence

During an unrelated GitHub issue-cleanup pass (5 parallel background agents, heavy short-lived
`gh`/`git`/`tsc`/`node` subprocess churn over ~50 minutes), system commit rose from the §8
baseline (5.72 GB) to a floor of **~7.4–8.3 GB** and did **not** revert once that work finished
and process count settled back to just this session's single `claude.exe` (same PID throughout,
private bytes barely moved: ~550MB → ~580MB).

**Where the growth landed, measured directly:**
- Sum of every process's private/paged memory on the machine: **5.09 GB**
- Total system committed: **8.03 GB**
- **Unattributed (no owning process): 2.93 GB** — bigger than all of AgentMux's own processes
  combined (`agentmux*` Private Bytes: only ~790-810 MB), bigger than `claude.exe` itself.

This unattributed bucket (kernel pool, page tables, and critically GPU-driver-committed pages)
is **exactly the mechanism #2218 described** — "driver-committed memory not reclaimed until
restart," charged to the System process (PID 4) rather than any visible process. **#2218 was
closed earlier in this same session** (see the issue-cleanup report,
`docs/status/STATUS_ISSUE_DISCUSSION_CLEANUP_2026_07_24.md` §1) on the evidence that its 3 fix
PRs (#2220/#2221/#2222) shipped in v0.54.4. This new data doesn't necessarily contradict that —
the fixes were about renderer-pool eviction on *pane close*, and this session did a lot of
*background-agent* churn, a different load pattern that may not be covered by those fixes — but
it's close enough to the same symptom that it needs to be checked, not assumed unrelated.

**Caveat:** this is one time-series snapshot, not yet a proven trend — no pre-burst "unattributed"
baseline was captured (only total system commit was tracked at that point). Telemetry v3
(`pf_telemetry_v3.ps1`, PID recorded at launch, writing `mem_sample_v3.csv`) now tracks
`AllProcessPrivateSumMB` and `UnattributedMB` every 10s specifically to turn this from a
one-off observation into a real trend.

**Recommended next step:** watch `mem_sample_v3.csv`'s `UnattributedMB` column over the next
normal work session. If it keeps climbing (independent of `claude*`/`agentmux*` process growth,
which this sample shows is NOT where the growth is), that's strong evidence #2218's mechanism
recurred or has an uncovered trigger path (background/subagent spawn churn vs. pane-close
eviction) — worth reopening #2218 or filing a fresh issue cross-linking it, with this doc as
the evidence trail. Do not reopen/refile on this single data point alone.

## 10. Trend confirmed — this is no longer a single data point (2026-07-24, ~7.5hr later)

`mem_sample_v3.csv` now has ~7.5 hours of continuous samples (05:09:42 → 12:38:01, one row
every ~13s). The shape is about as clean as this kind of data ever gets:

| Metric | 05:09:42 | 12:38:01 | Δ over 7.47h |
|---|---|---|---|
| `SystemCommittedMB` | 7,817.7 | 13,914.6 | **+6,096.9 MB** |
| `ClaudePrivateBytesMB` | 566.2 | 585.9 | +19.7 MB (noise) |
| `AgentmuxPrivateBytesMB` | 796.8 | 914.4 | +117.6 MB |
| `AllProcessPrivateSumMB` (every process, machine-wide) | 5,190.4 | 5,301.4 | **+111.0 MB** |
| `UnattributedMB` | 2,627.3 | 8,613.2 | **+5,985.9 MB** |

**~98% of the entire system-commit increase over 7.5 real hours landed in `Unattributed`** —
memory owned by no process at all. Every actual process on the machine, including every
AgentMux process and this pane's own `claude.exe`, combined grew by only ~111 MB in the same
window — noise-level. This rules out a conversation-transcript/context-growth explanation for
the bulk of the growth (§5's leading hypothesis going into this session) — the growing thing is
not `claude.exe`'s own memory, or the AgentMux app's own memory. It's the kernel-pool/
GPU-driver-commit bucket #2218 was originally about.

**Rate: ~800 MB/hour, sustained.** Extrapolating from the current ~13.9 GB to the user's
originally-reported 69 GB scenario: ~55 GB more growth ÷ 0.8 GB/hr ≈ **~69 hours (~2.9 days) of
uptime** — a very plausible real-world timeframe for a persistent dev pane left running across
a few days, which matches how the user actually uses AgentMux.

There was one transient burst around 09:13–09:48 (a second `claude.exe` instance briefly
appeared, `AllProcessPrivateSumMB` spiked to ~7,996 MB) correlating with this session's own
`task dev`/`cargo build` activity — but the linear climb continued at essentially the same slope
before, during, and after that burst. **The steady background growth is not explained by that
one burst** — something is leaking continuously, independent of what this session was actively
doing at any given moment.

**Updated recommendation: this now clears the bar for action, not just more watching.** Given
7.5 hours of clean linear-trend data with a textbook #2218-mechanism signature (system commit
growth with zero corresponding process-level growth), the next step should be to either reopen
#2218 or file a fresh issue cross-linking it and this doc — flagged to the user
(2026-07-24 session) as a recommendation, not yet actioned unilaterally. If a fresh issue is
preferred over reopening (since the trigger pattern here — hours of otherwise-idle-ish uptime,
not pane-open/close churn — may be a distinct path through the same underlying driver-commit
mechanism, not proof the original #2218 fixes regressed), title it something like "System
commit charge climbs ~0.8 GB/hour independent of any process's own memory — recurrence or new
trigger for #2218's driver-commit mechanism" and attach this doc's §9-§10 data directly.

## 11. #2218 reopened; this session's data doesn't fit the July 16 renderer-count model

Issue #2218 was reopened with §9/§10's data. Before researching a fix, read the two retros
#2218 is actually built on (`retro-commit-charge-pagefile-growth-2026-07-02.md`,
`retro-commit-restart-reclaim-2026-07-16.md`) — previously only referenced by name in this
doc, now read in full. Their finding, from a **decisive controlled restart experiment**:
driver-committed memory is **live, not leaked** — tied 1:1 to renderer *count*
(~1 GB of invisible driver commit per renderer process), released cleanly the moment a
renderer's process dies. It ratchets up over a session only because renderers are
pooled/never torn down on pane close — B.4/B.5 (this issue's merged fixes) target exactly
that: deterministic teardown + pool eviction when live count exceeds target. **Also
established: `--disable-gpu` is explicitly rejected as a mitigation** (owner policy,
2026-07-16 — "GPU stays enabled except when there is absolutely no option") — not
re-litigating that here.

**This session's data doesn't fit that model.** One pane, one `claude.exe`, the entire
7.5-hour window — no new panes/windows opened or closed. `AgentmuxPrivateBytesMB` stayed
flat (~795→915 MB), which per the July 16 rule of thumb ("each renderer adds both its own
visible private bytes AND ~1GB invisible driver commit") means renderer *count* did not
meaningfully grow either. Yet `UnattributedMB` still climbed ~6 GB. **If driver commit were
purely a function of renderer count, it should have been flat too — it wasn't.**

**Working hypothesis: a second, distinct trigger** — growth proportional to compositor
*activity* within an already-open renderer (this pane streamed a large volume of agent
text output over the window), not to renderer count. This is a plausible fit for a
recognized *class* of Chromium bug — GPU-memory-buffer / SharedImage handles not being
fully released on ordinary frame-to-frame reuse (as opposed to at renderer
creation/teardown) — which has recurred across Chromium versions, not as one canonical bug
with one fix:
- [chromium.org/470234](https://bugs.chromium.org/p/chromium/issues/detail?id=470234) —
  "Large memory leak in GPU process"
- [issues.chromium.org/41125802](https://issues.chromium.org/issues/41125802) — "GPU
  process memory usage too high"
- A documented offscreenCanvas GPU-buffer-not-deallocated regression in Chromium 93-95

AgentMux currently bundles **CEF 148** (`agentmux-cef/Cargo.toml`) — recent, not an obvious
"just upgrade" fix, but worth checking 148.x patch notes for anything GMB/SharedImage-related
before ruling it out.

**No single flag/config fix found** after a real search pass (shader-cache flags, DXGI
shared-handle management, DirectComposition swapchain leaks, GpuMemoryBuffer leaks all
turned up as *known problem categories* in Chromium's own history, not as a solved-and-
documented fix applicable here) — this needs AgentMux-side instrumentation, not a
one-line config change.

### Recommended next steps (ranked, none done yet — for the next session/owner decision)

1. **GPU memory trace, not just process-level commit.** Chromium/CEF expose GPU memory
   tracing (`docs/memory-infra/probe-gpu.md`) — capture one during a long single-pane,
   no-new-panes session to see which GMB/SharedImage *category* actually grows. This is the
   single highest-value next diagnostic (matches this project's own "measure before
   architecting" discipline from the July 2 retro) — everything above this point is still a
   hypothesis, not a confirmed root cause.
2. **A controlled activity-vs-idle experiment**, mirroring the July 16 restart experiment's
   rigor: hold pane/renderer count exactly constant (one pane, nothing opened/closed) and
   compare `UnattributedMB` growth rate during a heavy-output-streaming period vs. an
   equal-length idle period. Confirms or refutes the "activity, not just wall-clock time"
   hypothesis directly — this session's own data wasn't a clean A/B (it mixed idle and busy
   periods) and can't distinguish "leaks per unit time" from "leaks per unit of rendered
   content" on its own.
3. **Pragmatic mitigation that doesn't touch the rejected `--disable-gpu` path**: B.4/B.5
   already proved renderer teardown cleanly reclaims driver commit. If (1)/(2) confirm an
   activity-proportional leak *within* a renderer's lifetime, a proactive background
   recycle of a long-lived pane's renderer (time-boxed and/or memory-pressure-triggered,
   not just on user-initiated pane close) reuses that already-validated reclaim path instead
   of requiring a new mechanism — a scope note for whoever picks this up, not a proposal to
   implement blind.
4. Cross-link this section from #2218 (done — see the issue's follow-up comment).

## 12. Correction — the activity-proportional hypothesis (§11) is refuted by this session's own data

§11's "growth tracks compositor activity, not renderer count" hypothesis was posted to #2218
as a working theory, not a verified fact. Rather than leave it unverified, joined the
continuous `mem_sample_v3.csv` telemetry against this pane's own srv-log activity events
(`blockfile:line_count` — fires on every chunk of content actually written/rendered;
`subagent spawned`; `AgentInput`) to test it directly. **Method note:** the CSV timestamps are
local (PDT, UTC-7); the first join attempt forgot the offset and produced a spurious
correlation — corrected before drawing any conclusion (always verify a timestamp join by
spot-checking one known instant before trusting the bucketed output).

**Result, bucketed into 10-minute windows over the full ~9-hour window (12:00–20:00 UTC):**

| Window type | Mean `UnattributedMB` growth per 10 min |
|---|---|
| Zero logged pane activity (spans **~4 straight hours**, 12:00–15:40 UTC) | **~118 MB** |
| Active buckets, excluding 2 build-burst outliers | **~125 MB** |
| Active buckets during the `cargo build`/`rustc` burst (16:10–16:40 UTC — an independent confound: parallel compiler processes churning, not this pane's own rendering) | 282–589 MB (2 outlier buckets only) |

**~118 MB/10min vs ~125 MB/10min is not a meaningful difference — the hypothesis in §11 is not
supported.** Nearly 4 straight hours with zero recorded content/message/subagent events in
this pane grew at essentially the same rate as busy stretches. Growth tracks **wall-clock
time the pane/window has been open**, not what's happening in it.

**Follow-up check on the natural sub-hypothesis (a continuously-animating "running" indicator
driving a constant compositor heartbeat even with no new content):** none of this session's
4 `mcp__agentmux__Shell`-tracked shells (which would show a pulsing `.running` row in
`ActivityDock`) were created before 16:15 UTC — i.e. **no tracked running-status row existed
in this pane at all during the confirmed-flat 12:00–16:00 UTC quiet window.** That specific
sub-hypothesis (an ActivityDock `shell-running-pulse` animation as the driver) is therefore
also not supported by this data, though it doesn't rule out some other always-on animation
(composer caret blink, window chrome, DWM's own baseline compositing of any visible window)
that this investigation didn't separately instrument for.

**Net effect on the recommended next steps (§11):** step 1 (GPU memory trace) becomes more
important, not less — specifically to check whether **present/frame counts are non-zero
during confirmed-idle hours** (if zero, the "constant compositor heartbeat" idea is dead too,
and the mechanism is something else entirely — e.g. a slow per-context handle leak that
accrues on a timer unrelated to rendering at all). Step 2 (controlled activity-vs-idle
experiment) is now largely **already answered by this retroactive analysis** — no need to
re-run it as a fresh synthetic test; the real 9-hour session already provided a clean natural
A/B. Step 3 (proactive renderer recycle) remains a viable pragmatic mitigation regardless of
which specific mechanism turns out to be true, since it doesn't depend on identifying the
exact leak site — it just periodically resets the one thing already proven (July 16 retro) to
reclaim commit cleanly.

Posted as a correction to #2218 rather than editing the earlier comment, so the investigative
trail (including the wrong turn) stays visible for whoever picks this up next.

## 13. A forceful process-tree kill did NOT cleanly reclaim driver commit (2026-07-24, later same day)

Separate from the tracing work above: this session ran a **second full AgentMux instance**
(`task dev`, needed to test/verify two unrelated bug fixes — PRs #2295/#2296) alongside the
original pane for an extended period, plus several full Rust release rebuilds and the 863 MB
trace capture from §11/§12. System commit climbed from the §12 baseline (~13.9 GB) to **~22 GB**
over that stretch — expected, not itself alarming, since running a whole second app instance is
exactly the kind of load the July 16 retro's renderer-count model predicts should raise commit
(11 extra `agentmux-cef` processes counted live, roughly matching "~1 GB invisible driver commit
per renderer").

**The test this enabled, unplanned but decisive:** stopped the second instance via
`mcp__agentmux__ShellStop` (a tree-kill of the whole process group — not the app's own graceful
quit path, which is how AgentMux Windows normally shuts down, and how the July 16 retro's own
teardown experiment closed the app). All 11 `agentmux-cef` processes confirmed gone within
seconds. Commit dropped from ~21.9 GB to **~19.9 GB and settled there** (checked again after a
full minute — stable, not still draining) — a real but modest ~2 GB reclaim, not the ~9-11 GB
the renderer-count/graceful-quit precedent would predict for 11 fewer renderers.

**This is new information, not yet explained:** either (a) a forceful process-tree kill
genuinely doesn't release GPU-driver-committed resources the same way AgentMux's own graceful
quit sequence does — plausible given this session independently found a real gap in that exact
graceful-shutdown path (§ the "quit watchdog: reducer desync" bug hit earlier today, unrelated
repo area, `agentmux-cef/src/wrr/win_event.rs`) — or (b) the renderer-count model's "~1 GB per
renderer, cleanly reclaimed" finding from July 16 doesn't generalize to every teardown path, only
the one that retro actually tested. Not investigated further this session — flagging as a
genuinely new, real variable (**teardown method**: graceful app-quit vs. forceful process kill)
for whoever continues this, worth a controlled A/B the same way July 16 did for renderer count.

**Current state, for the record:** system commit ~19.9 GB as of this write-up, telemetry
(`mem_sample_v3.csv`) still running continuously since this morning's restart baseline.

## 14. Decisive: it's not kill method — driver commit is pooled per GPU adapter, not per process

§13 left one variable untested: whether a *graceful* close (app-initiated, not a process-tree
kill) would reclaim cleanly with a second instance concurrently running. User's own framing was
exactly right and worth testing directly: **"it needs to clear memory regardless of how you kill
it, both need to be covered, when the app is in the wild."** Ran the controlled A/B:

1. Relaunched a second `task dev` instance alongside the original (9 `agentmux-cef` processes,
   commit at **21.46 GB**).
2. Closed it via `Process.CloseMainWindow()` — sends `WM_CLOSE` to the main window, the *exact*
   signal a user clicking the title-bar X triggers. This is AgentMux's real graceful-shutdown
   path, not a simulation of it (no RPC/API involved at all).
3. All `agentmux-cef` processes for that instance confirmed gone within 30s.
4. Commit after a full minute settling: **20.52 GB** — only **~0.94 GB reclaimed**. Slightly
   *less* than §13's forceful-kill test (~2 GB reclaimed under otherwise-identical conditions:
   same two-instance setup, same original instance left running throughout both tests).

**Both teardown paths — forceful kill and genuine graceful app-quit — fail to reclaim the vast
majority of a second instance's driver commit, when a first instance is still running.** This
rules out "teardown method" as the variable entirely. What's common to both failures: a
**second, concurrent client of the same GPU adapter was still alive** when the tested instance
closed. The July 16 retro's clean, complete reclaim was measured with exactly one AgentMux
instance ever touching the GPU at a time — nothing tested there ever exercised the
multi-instance case, so its "closing AgentMux cleanly releases everything" conclusion doesn't
generalize to it.

**This is the headline finding of the whole investigation, not a side note.** It directly
explains a real, mainstream, explicitly-documented usage pattern (this repo's own CLAUDE.md:
*"AgentMux is designed to run multiple instances simultaneously"*) — not an edge case, not
something only this investigation's own dev/test setup would hit. Any user running two or more
AgentMux windows/instances (a portable + a dev build, two portables, testing an old version
alongside a new one — all explicitly supported per that same doc) will accumulate driver commit
that individual-instance restarts cannot clear, only a **full logoff/reboot** (which resets the
GPU driver's own adapter-scoped state) or **closing every concurrent instance simultaneously**
(untested this session — worth checking specifically, since it would confirm the adapter-pooling
theory even more precisely: does closing the *last remaining* client finally release everything,
the way July 16 always effectively tested?).

### Next steps (ranked, none done yet)

1. **Confirm the "last client releases everything" prediction** — with only the ORIGINAL
   instance left (this session's own pane), gracefully close *that* one too (or, safer, note its
   commit floor now and compare against a subsequent full restart) and see if commit finally
   drops to the ~8 GB clean-restart baseline. This is the single most direct confirmation
   available and doesn't require any new tooling.
2. **This reframes the mitigation direction entirely.** §11's "proactive renderer recycle"
   proposal (recycle a long-lived pane's renderer periodically) would NOT help — recycling one
   instance's renderers while ANOTHER instance's renderers keep the adapter pool alive changes
   nothing. The two real options are: (a) accept this as a Windows/GPU-driver-level constraint
   inherent to running multiple Chromium-embedding apps concurrently against one adapter, and
   communicate/mitigate around it (e.g. a status-bar note when multiple instances are detected
   running, or explicit guidance to fully close all instances periodically) — or (b) investigate
   whether a *specific* AgentMux GPU/CEF configuration choice (a shared vs. per-instance
   `RequestContext`, D3D device sharing flags, ANGLE backend choice) is causing MORE
   adapter-level pooling than a "normal" multi-instance Chromium-embedding app would see, which
   would make this partially addressable in code rather than purely a Windows/driver limitation.
3. **Check whether this reproduces on a machine with a different GPU vendor** (this machine's
   specific driver was never identified in this investigation) — adapter-level pooling behavior
   can be driver/vendor-specific, and confirming/refuting on AMD or Intel graphics (vs. whatever
   this machine has) would clarify whether (b) above is worth pursuing or this is fundamentally
   a Windows WDDM/vendor-driver characteristic outside AgentMux's control.

## 15. Root cause confirmed: this is NOT an AgentMux defect — plain multi-instance Microsoft Edge reproduces it identically

Per §14 next-step (2), ran two more targeted experiments before reaching for the "different GPU
vendor" test — both fast, safe, and fully reversible.

### 15.1 Surgical recycle tests (negative results — rule out two mitigation hypotheses)

With one throwaway `task dev` instance running alongside the original (commit ~21.6 GB, two
concurrent AgentMux instances):

- **Killed only that instance's GPU process by PID** (not the whole instance). The app
  self-healed exactly as designed — a fresh `--type=gpu-process` child spawned automatically
  within ~2s, confirming Chromium's transparent GPU-process-restart handling works correctly in
  AgentMux. Commit **did not move** (22.16 → 22.15-22.56 GB, noise-level). The pooled/unattributed
  commit survives a full GPU-process restart, so it isn't held by that process's own allocations —
  it's genuinely adapter/driver-level state, external to any one process's lifetime.
- **Killed two of that instance's five renderer (pool-window) processes by PID.** App tolerated
  it fine (pool windows are designed to be disposable/refillable). `AllProcessPrivateSumMB`
  dropped ~200-400 MB as expected (each renderer's own attributed memory released cleanly) but
  **`UnattributedMB` did not drop** (15,084 → 14,967-15,086, noise-level) — actually ticked up
  slightly on the next sample. Confirms the pooled portion isn't apportioned per-renderer either.

Both results independently point the same direction: whatever is pooled lives below any single
process AgentMux controls — reinforcing §14's adapter-level-pooling theory and ruling out
"periodic GPU-process recycle" and "evict idle renderer" as fixes for the *unattributed* portion
specifically (pool eviction is still worth doing for the *attributed* portion — see §15.3).

### 15.2 The decisive test: does plain Microsoft Edge show the same thing?

This is the test that actually answers §14 next-step (2) — "is this an AgentMux-specific GPU/CEF
misconfiguration, or a general Windows/Chromium characteristic?" — without needing a different
machine. `msedge.exe` is a completely separate Chromium-family codebase from AgentMux's CEF host,
built and shipped by a different team, sharing nothing but "runs on the same Windows GPU driver."
If it shows the identical partial-reclaim-only pattern, AgentMux's own code is exonerated as the
cause.

Method: launched two **fully isolated** Edge process trees via `--user-data-dir=<distinct temp
dir>` (the same mechanism that makes AgentMux's separate instances separate — distinct profile
directories force genuinely separate browser-process trees, each with its own GPU process, not
just separate tabs sharing one GPU process). Confirmed via `Win32_Process` enumeration: two
completely disjoint trees (`gpu-process` PIDs 10996 and 1908, ~15 renderers each — Edge's default
profile prewarms far more background renderers than AgentMux does).

| Step | SystemCommitted | AllProcessPrivate | **Unattributed** |
|---|---|---|---|
| Before any Edge instance | ~21.2 GB | ~7.0 GB | **~14.2-14.5 GB** |
| Both Edge A + B running (settled) | ~23.8 GB | ~8.7-9.2 GB | **~15.0-15.1 GB** |
| Edge A force-killed, **B still running** (settled) | ~22.6-22.7 GB | ~7.6-7.7 GB | **~14.97-14.98 GB** |
| Edge B also force-killed — **zero Edge processes left** (settled, 4+ min) | ~21.4-21.5 GB | ~6.6 GB | **~14.87-14.9 GB** |

Two Edge instances together added ~800-900 MB of unattributed commit. Closing the *first* of the
two, while the second stayed open, reclaimed essentially **none of it** (~15,080 → ~14,975 MB —
the same partial-reclaim-only shape AgentMux showed in §13/§14, down to the same order of
magnitude). This is not an AgentMux code path, not AgentMux's CEF init flags, not AgentMux's
ANGLE/GPU switches — it is Microsoft's own shipping browser exhibiting the identical behavior.

A second finding from the same run, worth flagging honestly rather than glossing over: after
**both** Edge instances were fully closed (zero `msedge.exe` processes remaining, confirmed),
unattributed commit settled at ~14.87-14.9 GB and held flat for 4+ minutes — **not** the ~14.2-14.5
GB pre-test baseline. A ~400-650 MB residue persisted past complete process teardown within this
observation window. This is within-noise-adjacent (the pre-test baseline itself ranged
14,232-14,502 MB, a 270 MB spread from unrelated background processes) but is *at minimum*
suggestive that "last client exits" doesn't guarantee *instant* full reclaim either — there may be
a slower driver-side trim/GC pass, or a genuinely sticky residue that only clears on logoff/reboot
(§14's original fallback hypothesis). Not conclusively resolved; flagged for whoever revisits this
with a longer observation window.

### 15.3 Conclusion — reclassifying this issue

**AgentMux is not leaking GPU-driver commit charge.** The pooled, per-adapter, partial-reclaim-
on-instance-close behavior documented across §13/§14/§15 is a Windows WDDM / Chromium
multi-process characteristic, reproduced identically by an unrelated, independently-built
Chromium-family application (Microsoft Edge) under the same "multiple separate profile
directories running concurrently" condition. There is no code-level fix available to AgentMux for
the *unattributed/pooled* portion — it lives entirely below the application layer, in driver/OS
territory AgentMux cannot reach from CEF's `on_before_command_line_processing` or anywhere else in
its own process tree. `--in-process-gpu` (the one architectural lever that could have avoided
per-instance GPU-process/adapter-session overhead entirely) was already tried and reverted in
v0.33.66 for an unrelated, worse failure mode (zombie white-screen with no recovery on GPU context
loss) — not a viable path to revisit for this.

**What IS actionable, and still worth doing** (separate from the unattributed/pooled mechanism
above): §15.1's renderer-kill test confirmed that each pool/tear-off renderer DOES hold a real,
fully-reclaimable chunk of *attributed* private-bytes commit (~90 MB+ per window, matching the
figure already measured and documented in `park_and_blank_window`'s doc comment in
`window_pool.rs`). `demote_promoted_pool_window`'s own doc comment (same file, line ~758) already
flags the gap directly: pool demote-cap tightening under pressure is refill-suppression only,
"without (yet) building an on-demand 'evict an already-idle pool window' primitive for the window
pool specifically (deferred — see the plan behind #2218)." That primitive remains unbuilt. It
would not touch the unattributed/pooled portion this status doc has been chasing, but it would
give AgentMux a genuine, own-code, shippable reduction in the *attributed* commit footprint of
idle pool windows under memory pressure — the legitimate remaining engineering task this
investigation surfaces, and it already has a design precedent to follow
(`demote_promoted_pool_window`'s strict-HWND-resolution-first, mutation-free-failure-path
discipline).

**Recommended framing going forward:** stop treating #2218 as a "find the leak" hunt — the leak
hunt is complete and came back negative for AgentMux's own code, positive for Windows/Chromium
platform behavior. The user-facing question becomes a product/UX one (how much to explain elevated
commit under multi-instance use, whether to nudge users to consolidate concurrent instances) rather
than an engineering bug. The one remaining piece of actual engineering work is the idle-pool-window
eviction primitive above, scoped as its own follow-up rather than bundled into this investigation.

## 16. RESOLVED — root cause: Audiosrv leaks ~1 pagefile-backed Section per second (2026-07-25)

§13-§15 are hereby **superseded**. Their GPU-pooling conclusions were measurement artifacts.
The user's pushback ("just using a different data dir means you cannot reclaim? that doesn't
sound right") was correct and triggered the re-verification that unraveled everything.

### 16.1 How the wrong conclusion fell apart

1. **Direct GPU counters refuted §14/§15.** Re-ran the two-instance Edge A/B test measuring the
   actual WDDM counters (`\GPU Adapter Memory(*)\Dedicated/Shared/Total Committed`) instead of
   the earlier proxy (`SystemCommitted − Σ process private bytes`). GPU adapter memory reclaimed
   **fully and cleanly** on every teardown — closing one of two instances returned the adapter to
   within noise of its pre-launch value. (An earlier contradictory reading was traced to a stray
   Edge process left behind by an incomplete kill — a testing-hygiene error, caught and redone.)
2. **The "unattributed gap" doesn't respond to browsers at all.** With clean measurement:
   two full Edge instances added ~3 GB commit, ~90% process-private, fully reclaimed on kill;
   the unattributed gap moved ±150 MB (noise) across launch AND teardown.
3. **The gap grows at a perfectly constant ~12 MB/min, 24/7,** regardless of workload — from
   2.0 GB (05:25 on 7/24) to ~20 GB (7/25 04:00) in near-identical ~230 MB steps every 20 min,
   through busy periods, idle periods, and overnight (full trace: `mem_sample_v3.csv`).
   Process churn was separately exonerated (300 process spawns → +16.8 MB).
4. **The historical rate predates all custom telemetry** (srv `mem_attribution` logs):
   7/20 ≈ 38 MB/min, 7/21-22 ≈ 26 MB/min, 7/23 ≈ 10 MB/min, 7/24 ≈ 12 MB/min. (The
   higher earlier rates include attributed growth from busier multi-instance/agent days; the
   constant unattributed component underlies all of them.)

### 16.2 The identification chain

- Handle-count sweep: **`svchost` PID 3432 held 84,289 handles** — ≈ 1 per second of machine
  uptime. Service: **Audiosrv (Windows Audio)**.
- `handle64 -s` breakdown: **83,908 of them were Section handles** (anonymous pagefile-backed
  shared memory). 84 K × ~205 KB ≈ 18 GB — the gap, exactly. Live growth confirmed at
  ~0.9 handles/sec, matching the commit slope (200 KB/s ÷ 0.98/s ≈ 205 KB/section).
- Sections are charged to **no process's private bytes** — invisible to Task Manager, Process
  Explorer per-process views, and every per-process counter. This is why weeks of
  process-attribution work could never find it.
- Ruled out as the triggering client (leak rate unchanged during each test): AgentMux's CEF
  audio service process (killed, respawned), process churn, audio device-change storms (event
  logs quiet), the custom PowerShell sampler (leak predates it), parsecd (CPU-implausible for
  a 1 Hz COM loop: <1 s total CPU in 24 h). The per-second *trigger* remains unidentified —
  but the *mechanism and remediation* are proven.
- **Traktor is entangled, not exonerated** (post-fix data point): against the fresh Audiosrv's
  ~0.4/s baseline, suspending Traktor for 120 s made the leak **4× FASTER** (197 handles/120 s
  ≈ 1.6/s), while the first suspension test (against the old saturated instance) showed no
  change. Something interacts with Traktor's always-active audio session about once a second,
  and when Traktor is frozen, that interaction appears to fail-and-retry, leaking more. Traktor
  (or its DJ-interface driver relationship) is the best remaining lead for the trigger.

### 16.3 The proof (and the reboot-free fix)

`Restart-Service Audiosrv` (elevated, user-approved UAC; peak meters checked silent first):

| | Before | After |
|---|---|---|
| Audiosrv handles | 85,084 | 584 |
| System commit | **27,079.6 MB** | **10,646.2 MB** |
| Unattributed gap | ~20,000 MB | 3,592 MB |

**16.4 GB reclaimed in ~9 seconds.** The machine's "inevitable" daily climb toward commit
exhaustion — the original trigger for this entire investigation line, the OOM crashes, the
restart ritual — is this leak.

### 16.4 Consequences and follow-ups

- **AgentMux's own code is exonerated** for the unattributed portion. The per-instance
  attributed footprint work (pool eviction etc., §15.3) remains valid product work but is
  unrelated to the machine's commit exhaustion.
- **Remediation available to the user today:** elevated `Restart-Service Audiosrv` whenever
  commit climbs (instant, no reboot; brief audio interruption — Traktor re-attached on its own
  in testing). Root-fix candidates: Windows Update / audio driver update for the DJ-interface
  stack (10 active render endpoints is an unusual topology and the likely trigger surface),
  and identifying the 1 Hz caller with an elevated ETW/ALPC trace if desired.
- **Product idea worth scoping** (separate issue): `mem_attribution` currently buckets only
  process-private commit, so this entire leak class is invisible to it. Adding
  `unattributed = SystemCommitted − Σ private − pools` as a logged metric — plus a heuristic
  alert on service handle-count anomalies (Audiosrv > ~10 K handles) — would let AgentMux
  detect-and-suggest ("Windows Audio service leak detected — restart it to reclaim N GB")
  for any user hitting this Windows bug. That converts this investigation into shipped value.
- The leak resumed at ~0.4/sec on the fresh Audiosrv instance, so accumulation continues
  (slower, possibly ramping back). The per-second trigger hunt is the remaining open thread,
  now cheap to pursue with the measurement kit built here (`audio_session_probe.ps1`,
  handle-rate sampling, suspend-bisection).

### 16.5 Retro notes (measurement discipline)

- The entire §13-§15 detour came from **trusting a derived proxy** (commit − process sum)
  without validating against a direct counter, then narrative-fitting partial reclaims to
  a plausible-sounding driver story. The correction came from (a) the user distrusting the
  conclusion, (b) swapping to primary-source counters, (c) noticing the constant slope.
- A constant growth slope independent of workload is a **timer signature** — check for it
  FIRST before building activity-correlated theories.
- One incomplete process kill (a lingering Edge tree) manufactured the single most misleading
  data point of the session. Always re-verify process-tree death before reading the after-state.
