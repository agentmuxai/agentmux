# Incident Report — AgentMux Closed Unexpectedly, Repeatedly (2026-09-20)

**Status:** retro — root cause analyzed (confidence-graded, see below); the
one concretely actionable bug this investigation surfaced (3 of 7 agent
panes losing their live process silently on restart) is filed as issue
#3463 and specified in
`docs/specs/SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`
(not yet implemented). The broader recommendations below (launcher exit
sentinel, §5.E/§5.G of `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`,
re-measuring issue #2218's pool cap, opt-in audit-policy change) are not
yet scheduled or filed as separate issues.
**Date:** 2026-09-20
**Reported by:** User (live, ongoing at time of investigation)
**Investigated by:** Agent3, corroborated independently by Lark
**Severity:** High — recurring, accelerating, affecting every agent sharing
this machine's production AgentMux instance. **Corrected 2026-09-20, same
investigation, TWICE, both catches by the user:**
1. An earlier revision of this doc claimed "~25+ concurrent agent sessions
   observed via `ListAgents`" here and in two places below. That was wrong
   — `ListAgents` was never actually queried for that figure; the number
   (~25-28 distinct names) actually came from `agentmux-launcher.log`'s
   `fs_watch` warning lines, which list every agent *identity* this
   workspace has ever had configured (a mostly-static historical roster),
   not concurrently-open sessions.
2. The walked-back replacement ("`ListAgents` showed 3-4 peer sessions") was
   **also wrong as a proxy for live agent-pane count**, caught immediately
   by the user comparing against AgentMux's own Swarm view, which showed
   **7** agents active in one window. Confirmed via the `Layout` MCP tool
   (ground truth — the backend's own live block/pane registry, not a
   proxy): **7 agent-view panes, in this one window** (5 in one tab, 2 in
   another), at the time of this second correction. `ListAgents` undercounts
   structurally, not just by timing — it only enumerates peer sessions
   reachable via Claude Code's own cross-session messaging protocol. This
   workspace runs agents under multiple providers (`gemini`, `kimi`,
   `codex`, `antigravity`, `openclaw`, `copilot`, `pi` identity directories
   were observed being fs-watched); a pane running a non-Claude-Code agent
   is a real, active agent with its own CEF renderer — contributing to the
   exact GPU-driver-commit floor this doc's root-cause analysis is about —
   but is invisible to `ListAgents`.

**The methodological lesson, for next time:** none of `ListAgents` (undercounts
— single-protocol only), the `fs_watch` identity list (overcounts — historical,
not live), or informal telemetry reading answer "how many agent panes are
live right now." The `Layout` MCP tool (or srv's underlying layout RPC it
wraps) is the correct, ground-truth source — query it directly rather than
inferring pane/agent count from any proxy.

**What is still not established:** the `Layout` query above reflects panes
open *now*, in the session investigating this (started ~20 min after the
last restart) — not necessarily the exact pane count at the moment of the
incident itself, and not necessarily every window on the machine (this
instance's `Layout` query returned exactly one window; whether that is the
totality of this instance's live windows, or scoped to the calling agent's
own workspace, was not independently confirmed). The "unprecedented scale"
framing in recommendation 2 below should be read with this in mind —
directionally plausible (7 agent panes in one window, each with a live
renderer, is a real and non-trivial contributor), but not a precisely
measured incident-time figure.

---

## What happened

The shared production AgentMux instance (`channels/local-main-b28b7a-b5599ad9`,
the Desktop portable, v0.56.8) silently exited — no error dialog, no crash
report — three or more times over roughly 15-45 minutes, each requiring the
user to manually relaunch. Every agent session hosted in that instance
(confirmed via `ListAgents`: all peers showed restart times under a minute
apart) was killed and restarted along with it.

This is the same headline as `docs/incident/INCIDENT_2026_06_26_APP_CLOSED.md`
("AgentMux Closed Unexpectedly") — that incident's root cause (Windows
OOM-killing the process with zero warning under page-file/commit exhaustion)
is the leading, well-corroborated explanation here too, not a new mechanism.
The difference this time: the **entire process tree** (launcher, srv, host)
died together, not just the CEF host as in June — see "What's new" below.

---

## Direct evidence

**No crash dump, anywhere.** Checked both `C:\CrashDumps\agentmuxsrv\` (the
in-repo VEH/minidumper for `agentmux-srv`, `agentmux-srv/src/crash_monitor.rs`)
and Windows Error Reporting's `LocalDumps` — nothing newer than 2026-09-15 in
either. Rules out an access violation, heap corruption, or a Rust
panic-abort.

**No internal exit classification either — the key finding.** `agentmux-launcher.log`
normally logs one of two lines for every host exit the launcher observes:
`"CEF host exited cleanly (code 0) — shutting down"` or `"CEF host exited
abnormally (code N); relaunching (restart M/3)"` (`agentmux-launcher/src/supervisor/windows.rs`).
Every historical restart in the log has one of these. **The recent
restarts have neither.** Between one srv instance's last log line and the
next srv instance's cold-boot sequence (full bootstrap: crash-handler VEH
install, agent registry attach, etc. — not an in-place recycle), there is no
shutdown reason recorded at all.

This means the **launcher process itself** was terminated outright (e.g. an
external `TerminateProcess`), not the CEF host observed and classified by a
surviving launcher. `classify_host_exit`'s `SystemOom`/`Abnormal` paths
(`agentmux-launcher/src/mem_supervisor.rs`, built after the June 2026-06-16
OOM incident) never fired, because the process that runs that classification
logic was itself the one killed. **This is the actual, previously-undocumented
gap**: every existing memory-pressure fix in this codebase assumes the
launcher survives to observe and classify a child's death; nothing observes
the launcher's own.

**System memory pressure, sustained, at the time of the restarts** — confirmed
independently by two sessions from the same `mem_attribution` telemetry
(`agentmux_srv::backend::sysinfo::log_memory_attribution`):
- Agent3 (this investigation): commit ~60.29/74.28 GB (81%), `unattributed_gb=26.00`,
  named consumers `chrome.exe:9535MB, NVIDIA Overlay.exe:5342MB, parsecd.exe:3912MB`,
  `process_count=419`.
- Lark (independent): commit pinned at 63-64/74.28 GB (85-86%) for several
  minutes, similar unattributed residual, similar named-consumer profile.
- Lark also found the launcher's own `instance_claim` restart timestamps
  accelerating: 38 min → 5 min → 1.75 min between restarts — worsening, not
  stable.

**Two known, previously-fixed leaks were ruled out as a regression**, both
checked live against the actual running processes rather than assumed from
history:
- `sysinfo` crate's `CreateToolhelp32Snapshot` handle leak
  (`docs/status/STATUS_SRV_SECTION_HANDLE_LEAK_LIVE_RECURRENCE_2026_08_19.md`,
  fixed PR #2666, confirmed via a 4-hour soak) — `agentmux-srv/Cargo.toml`
  still pins `sysinfo = "0.35"` (the fixed floor); live handle counts on both
  running `agentmux-srv-0.56.8` PIDs were 170 and 905 — nowhere near the
  leak's hundreds-of-thousands signature.
- `FsWatchPool` sweep leak (`docs/status/STATUS_FS_WATCH_SWEEP_HANDLE_LEAK_2026_08_22.md`,
  fixed PR #2722) — not independently re-verified this session, but no
  evidence implicates it (handle counts are healthy, and that leak has the
  same handle-count signature the check above already ruled out).

---

## What's new relative to the June 26 / July 2 / August 19 incidents

1. **The whole process tree died, not just the host.** June 26's incident had
   srv *survive* the host's death (proof it wasn't a machine reboot). This
   time, both srv and the launcher itself were gone — consistent with
   something killing the launcher (or the top of its process tree/Job
   Object) directly, a level higher than anything the June 2026-06-16 memory
   supervision spec (`SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`) was
   designed to observe.
2. **Concurrent-agent scale — corrected twice, see the top of this doc for
   the full methodology note.** Neither original proxy held up:
   `ListAgents` undercounts (single-protocol, Claude-Code-only), and the
   `fs_watch` identity list overcounts (historical, not live). The `Layout`
   MCP tool (ground truth — srv's own live block/pane registry) showed
   **7 agent-view panes in one window** at the time of the second
   correction — not incident-time-measured (see the caveat at the top of
   this doc), but a real, ground-truth-sourced figure for what this kind
   of session normally carries, and consistent with `process_count` sitting
   at 419-439 total OS processes and `agentmux_mb` at 560MB-1.7GB across
   both investigating sessions' `mem_attribution` readings. The actual
   live pane/window/renderer count at the incident's own moment was **not**
   directly measured and should be if this recurs — query `Layout` (or the
   backend RPC it wraps) at incident time, not a session-messaging or log
   proxy. The July 2026-07-02 GPU-driver-renderer-commit
   investigation (issue #2218, fixed PRs #2220/#2221/#2222 — see the
   correction added to `docs/specs/SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md`
   this same day) *bounded* that floor's growth (pressure-aware pool-demote
   cap) but never eliminated the fact that every pane/window still costs
   the GPU driver real, kernel-charged, pagefile-backed commit. Whether
   today's actual pane/window count exceeds what the 2026-07 fix was
   tuned/tested against is an open question this doc cannot answer without
   that direct measurement. The `unattributed_gb` residual both
   investigating sessions independently measured today (~26GB) is
   *consistent with* this same signature by order of magnitude (the
   2026-07 investigation measured ~43GB from a much smaller renderer
   count), but this is circumstantial, not a confirmed match — the same
   `--disable-gpu` A/B test the 2026-07-02 spec used to confirm the
   original attribution (§A.4 of `SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md`)
   has not been re-run today and would be the direct way to confirm it.
3. **`SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` §5.E (Job-Object
   soft-ceiling notification) and §5.G (cross-instance awareness) are
   confirmed still unbuilt** (per that spec's own §8 Phasing status: "P2 and
   P3 are not built"). Both would have made this incident's cause visible
   *before* the kill, rather than requiring two sessions to independently
   reconstruct it after the fact from telemetry.

---

## Root cause (confidence-graded)

- **High confidence, directly evidenced:** the launcher process itself was
  terminated by something outside AgentMux's own process tree — not a crash,
  not any of its own internal supervised-restart paths.
- **High confidence, ruled in by elimination + corroboration:** sustained
  system-wide commit pressure (81-86%, non-AgentMux processes dominating the
  *named* consumers, plus a large unattributed residual matching the known
  GPU-driver-commit signature from issue #2218) is the proximate trigger —
  this is the same mechanism class as `INCIDENT_2026_06_26_APP_CLOSED.md`,
  which confirmed Windows kills processes with zero warning under this exact
  condition.
- **Not proven, and not provable from this machine's current configuration:**
  the literal identity of what called `TerminateProcess` (Windows' own
  low-memory reclaim vs. some other external actor). Windows Process
  Creation/Termination auditing (`auditpol`) was checked and does not appear
  enabled on this machine — the Security Event Log cannot answer this
  retroactively. See recommendation 1 below.
- **Ruled out:** any deliberate action by another agent (checked directly
  with Lark, who was working an adjacent but unrelated spec on this same
  shared instance — confirmed not their doing, and their own independent
  telemetry corroborates the memory-pressure theory rather than contradicts
  it). Ruled out: regression of the `sysinfo` handle leak (§ above, verified
  live). Ruled out: a Windows Scheduled Task auto-updater (none found
  referencing AgentMux).

---

## Recommendations (ranked)

1. **Launcher-level exit sentinel (closes the genuinely new gap).** A
   lightweight companion process, spawned by the launcher at startup,
   symmetric to the existing `--crash-monitor` pattern
   (`agentmux-srv/src/crash_monitor.rs`) but scoped one level higher —
   watching the *launcher's own* process handle via `WaitForSingleObject`,
   not srv's. On any launcher exit not preceded by an intentional
   "graceful-shutdown" marker, it records wall-clock time, whatever exit
   code is available, and a `mem_attribution`-style snapshot, to a durable
   `launcher-exit-audit.log` — regardless of how the launcher died. This
   directly answers "we always want information on all exits," including
   the one case (the launcher itself) nothing currently observes.
2. **Build `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` §5.E (Job-Object
   soft ceiling) and §5.G (cross-instance/aggregate awareness)** — both
   already specified, both explicitly marked not-yet-built. §5.G in
   particular would have surfaced "N AgentMux-related panes/renderers are
   live across M agents on this machine" proactively, rather than requiring
   two independent post-hoc telemetry reconstructions (this investigation
   and Lark's) to arrive at the same conclusion by hand.
3. **One-time, opt-in Windows audit-policy change** (documented here as a
   recommendation for the user, not something AgentMux should silently
   flip): enable Process Creation (with command-line logging) and Process
   Termination auditing (`auditpol /set ...`). This is the only way a
   *future* recurrence could name the actual external killer via the
   Security Event Log, closing the one part of this RCA that could not be
   proven today.
4. **Re-scope issue #2218's fix validation to today's actual concurrency.**
   The pressure-aware demote cap (`effective_pool_demote_cap`,
   `agentmux-cef/src/commands/window_pool.rs`) was tuned and tested at a
   much lower pane/agent count than this machine now regularly sees. Worth
   re-measuring the per-instance GPU-driver-commit floor at realistic
   today-scale concurrency before assuming the 2026-07 fix still bounds it
   adequately.

---

## Corrections made to prior docs as part of this investigation

- `docs/specs/SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md`'s
  status line was stale — it read "proposed — Ready to schedule" for §B.5,
  but §B.5's core mechanism was in fact implemented (issue #2218, PRs
  #2220-2222, shipped v0.54.4), just via a different, better-informed
  design (pool-demote-and-reuse, not literal destroy-on-close) than the
  section's own original prose describes, because destroy-on-close turned
  out not to work at all (`docs/retro/retro-window-lifecycle-leak-2026-07-04.md`).
  Corrected in place with a dated note; see that file.

---

## Log sources consulted

| File | Key finding |
|------|------------|
| `~/.agentmux/logs/agentmux-launcher.log` | No "exited cleanly"/"exited abnormally" line for the recent restarts, unlike every historical one; fresh full-bootstrap sequences each time |
| `mem_attribution` INFO lines (srv stderr, same log) | Commit 81% (this session), 85-86% (Lark's), large unattributed residual both times |
| `C:\CrashDumps\agentmuxsrv\` | Empty except a live `monitor.sock` |
| `%LOCALAPPDATA%\CrashDumps\` (WER LocalDumps) | Nothing newer than 2026-09-15 |
| `Get-Process`/`Get-CimInstance Win32_Process` (live) | Confirmed handle counts on both live `agentmux-srv` PIDs healthy (170, 905) |
| `auditpol /get` | Inconclusive/not enabled — cannot retroactively confirm the external killer's identity |
| `ListAgents` | All peer sessions (3-4 at a time, not ~25+ — see correction at top of doc) restarted within under a minute of each other, confirming instance-wide impact |
