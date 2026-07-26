# Bashwrap, the Dock, and the Process Broker — a Seventh Mechanism Nobody Wired Up

**Date:** 2026-07-26
**Author:** AgentA
**Status:** Decided — open questions in §6 resolved 2026-07-26 (see §7); target architecture specified.
Grounded in two fresh, personally-reproduced incidents from today plus this codebase's own prior-art trail.
No implementation in this PR.
**Ground truth basis:** `agentmuxai/agentmux` `main` at commit `6962f81f`, pulled fresh for this report.
**Related (read in full while preparing this report, not just cited secondhand):**
- [`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md`](REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md)
  — the six existing process-liveness mechanisms + the proposed Process Broker. **This report's central claim
  is that this six-mechanism inventory is incomplete: `agentmux-bashwrap` is a seventh, structurally isolated
  one, invisible to that report's own research pass because it sits outside `blockcontroller`/`process_tracker`
  entirely.**
- [`REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md`](REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md)
  — identified the `run_in_background` threading gap (§6b) from the frontend/dock side. This report grounds
  that gap at its actual source: the wrapper binary that executes every Bash call literally cannot see that
  flag.
- [`REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md`](REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md)
  — ties dock/swarm/pane-status together as one initiative.
- [`docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md`](../retro/retro-persistent-agent-working-status-stuck-2026-07-16.md)
  (the "Agent1" incident) — an agent's dev stack read as stuck for ~12h because a heartbeat loop, added
  specifically to defeat bashwrap's idle-kill, also defeated the frontend's *unrelated* liveness watchdog.
  This report treats that incident as the **first** occurrence of the pattern it independently reproduces
  today as the **second**.
- [`docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md`](../retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md)
  and [`SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20.md`](SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20.md)
  — two more bashwrap-adjacent fixes, each scoped narrowly to its own symptom.

**Motivation:** while manually testing PR #2304 today, my own `task dev` launch (via the sandboxed Bash
tool, `run_in_background: true`) was silently killed 10 minutes in by `agentmux-bashwrap`'s idle-output
watchdog — not a real crash, but I initially couldn't tell the difference, because nothing surfaced the
kill anywhere. The user's framing on seeing this — "we run into bashwrap issues so often," assumed-but-
unconfirmed coupling to "the long-running process reducer state," and "the docked long-running processes
don't act deterministically" — is the brief for this report: assess whether that assumption is true, and
where the actual architecture stands.

---

## 0. Executive summary

The assumption in the brief is almost exactly backwards, which is itself the finding: **bashwrap is *not*
closely coupled to the long-running-process/reducer/dock/Swarm machinery — it has *zero* coupling to any
of it**, despite being the single highest-frequency code path in the entire system for "an agent runs a
shell command." Every ordinary `git status`, every `npm test`, and — as reproduced today — every
`run_in_background: true` dev-server launch issued through Claude's *native* Bash tool goes through
`agentmux-bashwrap exec`, a small, standalone, one-shot PTY wrapper that:

- has **no knowledge of `run_in_background` at all** (§1.2 — traced to the literal struct that drops the
  field on the floor),
- is invisible to all six of the process-tracking mechanisms `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK`
  catalogued (§2 — confirmed by grep, zero references),
- never appears in the ActivityDock, Swarm pane, or the proposed Process Broker's scope, and
- kills any command producing zero stdout for 600 seconds, indistinguishably whether that silence means
  "hung" or "a GUI app that's supposed to run for hours" (§1.3).

This is a **second, independent, uncoordinated liveness heuristic**, sitting entirely outside the six the
Process Broker report already flagged as fragmented — and it is not hypothetical. §3 reproduces it live,
today, in a manner that lines up almost exactly with the ~12-hour Agent1 incident from ten days ago (§3.3):
different agent, different session, same root cause, same category of user-visible symptom (something that's
actually fine reads as broken, with no way to tell from the UI), independently rediscovered because nothing
from the first incident changed how bashwrap or its siblings behave.

Separately, **there are two entirely independent "run a shell command" execution surfaces** in this codebase
— `agentmux-bashwrap` (behind Claude's native Bash tool) and `ShellNodeRunner`/`shell_node.rs` (behind
AgentMux's own `Shell` MCP tool, the thing that actually feeds the dock) — with different languages of
failure, different timeout policies (600s idle-kill vs. none at all), and a documented history of the *same*
class of bug (console-window flashes, `SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20.md`) being found
and fixed **twice**, once per surface, because neither shares infrastructure with the other. That duplication
is exactly the shape the Process Broker report describes for OS-process tracking (§3.0.3's "auth domain still
has an unfixed duplicate" caution) — found again here by coincidence, not because anyone went looking for it
specifically in this domain before today.

The "docked long-running processes don't act deterministically" half of the brief is real, but for a
different, better-diagnosed reason than "coupled to bashwrap": today's dock/reducer genuinely does model only
one axis (turn-phase) and treats liveness with several different, disagreeing clocks (§4). Bashwrap doesn't
cause that non-determinism — it's a parallel, unrelated failure mode that happens to *look* like the same
symptom from the outside (a healthy long process reads as dead), which is likely exactly why the two got
conflated in the brief.

---

## 1. What `agentmux-bashwrap` actually is

### 1.1 A one-shot PreToolUse rewrite, not a persistent service

`agentmux-bashwrap/src/main.rs`'s own module doc states its shape precisely: two subcommands, `hook` (reads a
Claude Code `PreToolUse` JSON payload on stdin, rewrites the command) and `exec` (runs the rewritten command
inside an owned PTY, streams output to the sidecar over HTTP, exits, propagating the real exit code as its
own — `main.rs:56-64`, fixed from an earlier always-exits-0 bug per its own comment, codex P1 on PR #804).

`agentmux-bashwrap/src/hook.rs`'s `build_response` is where every native Bash tool call gets intercepted
(`hook.rs:38-104`): it base64-encodes the original command into a rewritten invocation,
`agentmux-bashwrap exec --tool-id=<id> --b64-cmd=<b64>`, which Claude then actually runs. This is **not** a
persistent process manager — each `exec` invocation is scoped to exactly one tool call, runs for however long
that call takes, and terminates. There is no `agentmux-bashwrap` daemon; the binary that killed my dev-server
launch today was one specific, disposable process instance, spawned fresh for that one Bash call.

### 1.2 It cannot see `run_in_background` — traced to the actual struct

`hook.rs:26-33`:

```rust
#[derive(Deserialize)]
struct PreToolUseInput {
    tool_name: String,
    #[serde(default)]
    tool_use_id: String,
    #[serde(default)]
    tool_input: Value,
}
```

`tool_input` is read once, for exactly one field (`command`, `hook.rs:67-71`). Whatever else Claude's own
Bash-tool schema carries in that payload — including `run_in_background: true`, the parameter that made my
dev-server launch a background task from Claude's own point of view — is present in the raw JSON `hook.rs`
reads from stdin but is **never extracted, never inspected, and never forwarded** into the `agentmux-bashwrap
exec` argv the hook emits. `REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md` §6b already found this
gap by grepping for `run_in_background` across the repo and getting zero hits outside prose (`"confirmed:
grep run_in_background across frontend/app, agentmux-srv, agentmux-bashwrap returns zero hits"`); this report
adds the *why*, at the exact line: the struct that parses the hook payload doesn't have a field for it, so
there is nothing downstream to wire even if someone tried.

### 1.3 The idle-kill: a real, working, but semantically blind heuristic

`agentmux-bashwrap/src/bash_wrap.rs:107`:

```rust
const DEFAULT_IDLE_KILL_TIMEOUT: Duration = Duration::from_secs(600);
```

The doc comment directly above it (`bash_wrap.rs:95-106`) is honest about the tradeoff: it exists because a
wrapped command can block forever on interactive input that will never come (a pager the PTY makes think
`isatty(stdout) == true`, per `docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md`), and killing the
whole process tree after 600s of *zero bytes of output* is how the wrapper avoids leaking forever. The
mechanism is correctly scoped as an idle timeout, not a total-runtime timeout — a command with continuous
output for hours is unaffected, only literal silence trips it. **The problem is not that this heuristic is
buggy; it's that it has no way to distinguish "silent because stuck" from "silent because it's a long-running
GUI process behaving exactly as intended,"** because nothing tells it which one it's looking at (per §1.2,
it structurally cannot know).

---

## 2. Confirmed: zero coupling to any of the six process-tracking mechanisms

Direct check against every mechanism `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md` §1
catalogued:

```
$ grep -rn "process_tracker\|CONTROLLER_REGISTRY\|reactive::\|blockcontroller" agentmux-bashwrap/src/
(zero matches)
```

`agentmux-bashwrap`'s only outbound integration is `wps_client.rs` — a thin HTTP client that publishes
`tool_chunk` WPS events carrying streamed output bytes, purely for the transcript's live tool-output
rendering (`wps_client.rs:1-16`). It does not register a PID with `process_tracker::AgentProcessRegistry`,
does not touch `blockcontroller::pidregistry`, does not participate in `HealthMonitor` or `watchdog.rs`'s
signals, and is not one of the four controller types (`shell`/`subprocess`/`persistent`/`acp`) that report's
§5.2 recommends must register with a future Process Broker. It sits **entirely outside** that report's frame,
not as an oversight in that report — a native Bash tool call isn't a controller-managed agent pane at all, so
there was no reason for that research pass to have looked here. But that also means: if the Process Broker
described in that report ships exactly as scoped, `agentmux-bashwrap`-executed commands remain exactly as
invisible to it as they are today. **This is the seventh mechanism, and it's the one with the most day-to-day
agent interaction of any of them.**

---

## 3. Fresh evidence: reproduced live, today, twice, by two different failure modes

### 3.1 Incident A — the 600s idle-kill, reproduced firsthand

Launched `task dev` (to manually test PR #2304) via the sandboxed Bash tool with `run_in_background: true`,
per this repo's own documented pattern for launching dev servers from an agent. The launcher/host/CEF process
tree came up cleanly (confirmed via `tasklist`, process-creation timestamps, and the app's own structured
logs — zero errors, clean paint). ~13 minutes later, a `<task-notification>` reported the background command
had "failed with exit code 1." The tail of its (delayed-flush) output read:

```
[bashwrap] command produced no output for the idle timeout and was terminated automatically (likely blocked
on a pager or other interactive prompt this wrapper can never answer, e.g. `git diff`/`log`/`show` auto-
paging output that doesn't fit one screen). Try `git --no-pager <cmd>` or `| cat` on future invocations.
```

`tasklist` confirmed the entire `agentmux-launcher.exe` → `agentmux-cef.exe` tree was gone. This is exactly
`bash_wrap.rs`'s idle-kill (§1.3) firing on schedule: Vite prints its ready banner, the GUI app then produces
zero further stdout for its entire (intentionally indefinite) lifetime, and 600 seconds later bashwrap
concluded — reasonably, given what it can see — that this looked like a hung pager and tore down the whole
tree. Nothing about this was a crash of AgentMux's own code; it was an external `TerminateProcess`/job-kill
from the very tool used to launch it, with **no dock entry, no notification, no trace anywhere in the UI** —
discoverable only by chasing process-creation timestamps and structured logs by hand, which is what this
report's investigation had to do.

**Fix used, not a systemic fix**: relaunched via `mcp__agentmux__Shell` instead (§1.4/§2's `ShellNodeRunner`
path) — which has no idle-kill at all (confirmed: zero hits for `idle`/`timeout`/`Duration::from_sec` in
`agentmux-srv/src/backend/shell_node.rs`) — and it ran the rest of the session without incident. This is a
correct workaround for *this* task, but it only exists because I happened to already know both surfaces
exist; nothing in the moment of failure pointed at it.

### 3.2 Incident B — a silent shell-dialect-mixing footgun, found diagnosing Incident A

Retrying via `mcp__agentmux__Shell` with `scripts\dev-agent.cmd TITLE=diag2 > /tmp/log 2>&1; echo DONE=$?
>> /tmp/log` (POSIX-shell syntax appended after a `.cmd` invocation) produced a bizarre, disconnected
failure: `task`'s own CLI printed its full task list, then `task: Task ";" does not exist`. The actual root
cause, confirmed by capturing output to a file and reading it directly: Git Bash's automatic `.cmd`/`.bat`
dispatch-to-`cmd.exe` behavior appears to forward the **entire raw command string** — including the trailing
POSIX-only `;`, `$?`, and `>>` — into `cmd.exe`'s own (incompatible) syntax, rather than isolating just the
`.cmd` invocation's own arguments. `cmd.exe` then parses `;` and `$?` as literal argv tokens, which
`dev-agent.cmd`'s `task dev %*` forwards straight into `task`, producing an error about a task named `;` that
has nothing to do with the actual mistake (mixing two shell dialects in one command line). Isolating the
`.cmd` invocation as the sole, complete command fixed it immediately (§3.1's successful relaunch).

This is a narrower, more mechanical finding than Incident A, but it belongs in the same report: it is a
**second, independent way the exact same operation (launch a long-running dev process from an agent) produces
a confusing, misleading failure** with no signal pointing at the real cause — this time not even bashwrap's
fault, but a consequence of there being multiple shell-dispatch layers (bash → Git Bash's cmd/bat auto-detect
→ cmd.exe → task's own CLI parser) that don't agree on where one command's syntax ends and another's begins.

### 3.3 Corroboration: this is not a one-off — it's the second time, ten days apart

`docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` documents Agent1's own dev stack
running for ~12 hours while its pane showed an ambiguous "Waiting…/Working…." That retro's root-cause section
identifies, almost in passing, the reason Agent1's setup *didn't* trip bashwrap's idle-kill: it had wrapped
its own launch in a deliberate heartbeat loop —

```bash
while true; do sleep 120; echo "[heartbeat] dev still alive $(date +%H:%M:%S)"; done &
```

— which the retro states outright: *"The agent almost certainly added this heartbeat to defeat the idle-kill
of its background process… not realizing it would also pin the pane's busy indicator on"* (citing
`agentmux-bashwrap/tests/idle_kill_full_process_tree.rs` directly). That single sentence is the whole finding
of this report's §1–§3, independently arrived at from the other direction ten days earlier: **an agent had
already learned, empirically, that launching a long dev-server task through the default Bash tool gets killed
by bashwrap unless you fight it with a fake heartbeat** — and that workaround then broke a *second*,
completely unrelated system (the frontend's `LIVENESS_RECOVERY_MS` watchdog, which reads the heartbeat's own
output as proof the *turn* is still active, not just the attached process). Today's incident is the same root
cause with no heartbeat workaround in place, so it manifested as an outright kill instead of a 12-hour
ambiguous hang — arguably the more honest failure mode, but still one with zero user-facing signal.

Two independent agents, ten days apart, hit two different downstream symptoms (silent kill vs. 12h stuck
status) of the identical upstream cause: **bashwrap's idle-kill has no channel to express "this process is
supposed to run indefinitely," so the only ways an agent can currently cope are to either accept the kill or
to hand-roll a fake-liveness workaround that corrupts an unrelated status signal.** Nothing changed about
bashwrap between the two incidents; there was no reason to expect a different outcome today.

---

## 4. The "docked long-running processes don't act deterministically" half of the brief

This part of the brief is correct, but the mechanism is different from — and unrelated to — bashwrap. The
prior reports already diagnose it precisely, so this section is a pointer, not new research:

- **Two independent liveness clocks that can disagree.** The Agent1 retro's own evidence: the backend's
  `session:last_activity_ms` (CLI-stdout clock) stayed frozen while the frontend's pane indicator stayed lit
  — i.e. the two signals genuinely disagreed about the same agent at the same moment, because a background
  task's own output can refresh one clock without the other (§ "Why the liveness watchdog never auto-
  recovered it" in that retro).
- **A status label with fewer states than the system has situations.** The retro's own Lesson 2: "Working…"
  is asked to mean four different underlying situations (generating / rate-limited / a healthy attached
  long-running process / a genuinely hung turn) — any two colliding reads as a bug, and the fix is more
  states, not a better watchdog.
- **The dock has already shipped one determinism bug of exactly this flavor and fixed it**: the subagent-flood
  issue (`REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md` §5) — one raw backend event per dock row
  instead of one row per logical call — was a real, shipped non-determinism (dozens of rows for 1-2 actual
  tool calls), independently found and fixed twice the same day by two different agents before converging on
  one fix. That pattern (same bug class rediscovered independently, twice, same day) is itself evidence that
  the underlying abstraction (`PinnedActivity`/`ActivityKind`) doesn't yet make "what counts as one call"
  obvious enough to get right on the first attempt.
- **A structurally identical failure shape, found today in a completely different subsystem.** While
  addressing PR review feedback earlier in this session (unrelated to bashwrap), a genuine bug was found and
  fixed in `frontend/app/view/agent/flows/launch-flow.ts`: a failed `ControllerResyncCommand` was logged in a
  catch block, but execution still fell through to an unconditional "Ready"/"Resumed" success notification —
  the code's own adjacent comment claimed this exact fallthrough was avoided, and it wasn't. This is the same
  *shape* of bug as the Agent1 incident and the dock-flood bug: a failure or edge-case path gets silently
  absorbed into the nearest "happy path" state because there was no explicit state for what actually
  happened. It's unrelated to bashwrap specifically, but it's a third independent data point for "this
  codebase's long-running/status-reporting code has a recurring pattern of not modeling enough states,"
  observed in three unrelated files in the same week.

**Conclusion for this section**: the dock/Swarm non-determinism and the bashwrap non-determinism are two
separate bugs that happen to produce the same *symptom* (a healthy long-running thing looks broken), which is
almost certainly why the brief assumed they were coupled. They aren't — but they rhyme, and a fix for one
category (explicit states over inferred ones) is the right shape of fix for both.

---

## 5. Recommendations

1. **Extend the Process Broker's scope (or explicitly, in writing, exclude it) to cover bashwrap-executed
   commands.** Per §2, the broker as currently scoped in `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK`
   would not see these at all. At minimum, `PreToolUseInput` (§1.2) should capture `run_in_background` from
   the hook payload and thread it into the `exec` invocation (e.g. a `--background` flag bashwrap can use to
   pick a different, longer, or absent idle-kill policy, and to emit a distinguishable "this was declared
   long-running" signal the frontend can dock). This is the same recommendation
   `REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md` §6b already made from the frontend side; this
   report grounds it with the exact struct/line that needs to change.
2. **Do not let a declared-background command be killed by surprise, ever, without a visible signal.**
   Whatever the eventual policy (longer timeout, no timeout, or a heartbeat *the wrapper itself* emits so a
   real dev process doesn't need the agent to fake one — directly obsoleting the Agent1-style workaround),
   the one invariant that matters most: if bashwrap decides to kill a process tree, that decision must be
   visible somewhere the user or agent can see it near-term, not discoverable only by process-list forensics
   after the fact (as both this report's Incident A and the Agent1 retro required).
3. **Investigate whether `agentmux-bashwrap` and `ShellNodeRunner`/`shell_node.rs` should converge on shared
   execution infrastructure**, rather than remaining two independently-maintained PTY/process-spawn
   implementations with their own, already-diverging bug histories (idle-kill exists on one, not the other;
   the identical console-window-flash bug was found and fixed twice, `SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20.md`
   §2.1 vs §2.2). This does not have to mean literal code sharing — the credential broker precedent
   (`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK`'s §3.0) shows a broker can sit *above* heterogeneous
   backing implementations without unifying them — but today neither surface even knows the other exists,
   which is a stronger claim than "heterogeneous," it's "uncoordinated."
4. **Make the dual-surface split legible to whoever is operating an agent, today, before any of the above
   ships.** Concretely: a one-line addition to this repo's own dev-launch guidance (`CLAUDE.md`'s "Launching
   task dev from an agent / MCP Shell" section already exists and already recommends `mcp__agentmux__Shell`
   — it should say *why* in one sentence: the sandboxed Bash tool's wrapper will kill a silent long-running
   process after 10 minutes; the MCP Shell tool won't). This is the cheapest possible mitigation and would
   have prevented both of today's incidents outright.
5. **Treat "docked non-determinism" and "bashwrap non-determinism" as two separate workstreams** (§4's
   conclusion) — the fix for the dock/Swarm side is already well-specified in the three existing reports this
   one builds on (two-axis pane status, one canonical `ProcessStatus`, a real batch query); nothing here
   changes that direction. This report's contribution is narrowly the bashwrap side, which those reports did
   not cover.

---

## 6. Open questions

1. **Should bashwrap gain its own liveness-heartbeat emission** (so a long-running wrapped process can prove
   it's alive without the agent needing to fake one, per Recommendation 2), or is the simpler fix just "don't
   idle-kill a command explicitly declared `run_in_background`" and leave truly-foreground silent hangs
   (the actual pager case bashwrap was built for) as the only thing the timeout still guards? The latter is
   smaller and directly closes both of today's incidents; the former is more general but a bigger lift.
2. **Does `ShellNodeRunner` need an idle-kill at all**, or is its current no-timeout behavior correct given
   it's only reachable via a deliberate, dock-tracked `Shell` tool call (i.e. the user/agent already declared
   "this is long-running" just by choosing that tool)? If so, that's an argument *for* Recommendation 1's
   `--background` flag being bashwrap's version of "the caller already told us," rather than inventing new
   policy.
3. **Who owns the decision in Recommendation 3** (converge bashwrap and `shell_node.rs`, or formally accept
   them as permanently separate with a documented contract each)? Given `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK`'s
   Process Broker is itself an open proposal, this may be best sequenced as a follow-on question for whoever
   picks that report up, rather than decided independently here.
4. **Is `agentmux-bashwrap/tests/idle_kill_full_process_tree.rs` (and the sibling test that flaked in CI on
   PR #2301 today, `run_via_pty_does_not_misclassify_fast_success_as_idle_timeout`) evidence that the
   idle/success race this test guards against is inherently timing-fragile** in a way that a design change
   (not just a tighter implementation) would resolve more durably than the current test-and-patch cycle? Not
   investigated deeply here — flagged because it surfaced, unprompted, in this same session's CI run.

---

## 7. Decisions

Resolving §6, in order, against the same industry precedent `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK`'s
§4 already grounded this codebase in (systemd/Kubernetes/supervisord/Docker), plus that report's own §3.0.3
principle of **graceful degradation over aggressive action under uncertainty**.

### 7.1 (§6.1) No heartbeat mechanism. Declared-background calls skip the idle-kill entirely.

Rejected: bashwrap emitting or requiring a synthetic heartbeat from wrapped processes. This would reintroduce
the exact ambiguity the pager-hang bug already exposed — inferring liveness from a signal the wrapper doesn't
actually control or understand — and it's the same anti-pattern that broke the Agent1 incident's *other*
system (a hand-rolled heartbeat kept one liveness clock alive while a second, independent one froze). Adding
a bashwrap-native heartbeat is a more sophisticated version of the same mistake, just moved one layer down.

**Decision:** once `run_in_background` is threaded through (per §5 Recommendation 1), a call declared
background **never** gets the output-silence idle-kill. The 600s heuristic exists specifically for the
foreground case — a caller blocked synchronously on a result, where 10 minutes of literal silence is
overwhelmingly a stuck interactive prompt, not legitimate work (per `bash_wrap.rs`'s own doc comment: a
command with real, continuous output is already exempt regardless of total runtime). That foreground rationale
does not transfer to a background call, where the caller explicitly isn't waiting and silence is the expected
steady state for a GUI app, a dev server, or a watch loop.

This mirrors Docker's healthcheck model more closely than systemd's watchdog model, deliberately: Docker's
healthcheck is opt-in and author-defined per container, not an ambient default every container must satisfy;
systemd's `WatchdogSec` requires the *service itself* to actively call `sd_notify`, which is closer to the
rejected heartbeat option. The caller's own `run_in_background` declaration is the simplest possible signal —
no new protocol, no new code in the wrapped process, just trusting information that already exists at the
call site and today gets silently discarded (§1.2).

**What bounds a background task's lifetime instead:** not an output heuristic, but the same job-object/
process-group lifecycle binding `process_tracker` (mechanism #2) already gets right for controller-managed
processes (`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK` §3.0.3 point 3) — tie the spawned child to the
owning session/pane's job object so it's reaped when that session ends, rather than judged on its output
pattern. Confirming bashwrap's children already inherit the right job membership (or wiring it if not) is an
implementation-time verification, not re-litigated here.

### 7.2 (§6.2) `ShellNodeRunner`'s no-timeout behavior is correct as-is — keep it, don't add one.

**Decision:** no idle-kill for `mcp__agentmux__Shell`-spawned processes. Choosing that tool at all is itself
the same kind of explicit declaration `run_in_background` is for bashwrap — the caller is stating "this is
long-running and I want it tracked," which is precisely why it already feeds the dock. Adding an output
heuristic here would reintroduce, on the one surface that gets this right today, the exact failure this report
opened with. The two surfaces converge on one **policy**, arrived at independently for each: *an explicit
declaration of long-running intent — however each surface's caller expresses it — suppresses any
silence-based kill decision.*

The only gap worth closing on this surface is the same one as §7.1's second half: confirm session/pane-
lifecycle cleanup is the actual backstop against a truly abandoned shell (one whose owning pane closed without
an explicit `ShellStop`), not an assumption. If a gap is found there, close it with lifecycle binding, not a
timer.

### 7.3 (§6.3) No code convergence. Converge on a shared policy + shared future registration contract.

**Decision:** do not merge `agentmux-bashwrap` and `shell_node.rs` into one implementation. Their operational
contracts are genuinely different in a way that matters: bashwrap must remain a fast, standalone binary
invocable directly from Claude Code's own `PreToolUse` hook with no RPC round-trip at startup (base64'd argv,
no daemon, §1.1); `shell_node.rs` is inherently backend-owned and RPC-managed, feeding the dock/tracking
system by design. Forcing these into one code path would compromise the constraint that makes each one work
for its actual caller — the same "don't copy the precedent literally where the domains genuinely differ"
caution `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK` §3.0.3 already applied to the credential-broker
analogy, applied here a second time between these two surfaces instead of between auth and process domains.

What *does* converge, per §7.1/§7.2's shared policy: both surfaces should, once the Process Broker
(`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK` §5) exists, register their declared-long-running/background
processes into it — bashwrap's `exec` emitting the broker's equivalent of `ProcessEvent::PidObserved` once it
knows a call is backgrounded, `shell_node.rs` continuing to do so as part of closing that report's own §5.2
controller-type coverage gap. This is sequencing, not new scope: it's an addendum to the Process Broker
initiative, owned by whoever picks that report up, not a separate initiative competing with it. Until that
broker exists, §7.1/§7.2's policy changes stand on their own and don't depend on it landing first.

### 7.4 (§6.4) The idle/success race is a structural tie-break problem — recommend `select!` bias, not another patch.

**Decision:** treat the recurring flake as a real, still-open structural issue, not fully closed by the prior
"idle/success misclassification race" fix (`bash_wrap.rs`'s history already shows one dedicated fix for
exactly this race, and it still flaked in CI today on an unrelated PR). `tokio::select!` picks pseudo-randomly
between two simultaneously-ready branches by design; if the idle timer and the child's exit both become ready
in the same instant, the current unbiased race can still choose the idle branch and misclassify a fast,
genuinely successful exit as an idle-kill. The fix is structural, not another round of timing adjustment:
give the exit-detection branch priority — either `select! { biased; wait_task => ..., idle_rx => ... }` so a
resolved exit always wins a simultaneous race, or check `wait_task`'s completion before treating an idle-timer
tick as authoritative. Either removes the coin-flip instead of narrowing the window it can land in.

This is lower-priority after §7.1: restricting the idle-kill to foreground-only calls (§7.1's decision)
already shrinks how often this race can even trigger in practice, since most multi-minute-silent commands in
this codebase's actual usage are backgrounded dev servers and watch loops, not foreground waits. The bias fix
is still worth doing — it's small and removes a real, demonstrated flake — but it's no longer gating on
§7.1/§7.2 landing first.

### 7.5 Resulting target architecture, summarized

One policy, expressed identically by both execution surfaces, each in the vocabulary already available to it:

| Surface | Signal that suppresses silence-based killing | What still bounds its lifetime |
|---|---|---|
| `agentmux-bashwrap` (foreground call, the default) | none — kill-on-600s-silence stays, unchanged | the idle-kill itself (with the §7.4 bias fix) |
| `agentmux-bashwrap` (`run_in_background: true`) | the caller's own declaration, once threaded through (§5 Rec. 1) | job-object/session-lifecycle binding, verified not assumed |
| `mcp__agentmux__Shell` (`ShellNodeRunner`) | choosing this tool at all — already true today, kept as-is | explicit `ShellStop` + session-lifecycle binding, verified not assumed |

No new heartbeat protocol, no forced code merge, no new timeout tuning exercise. The two surfaces stay
separate implementations permanently, converging only on (a) this shared "declared-intent suppresses
inference" policy and (b) a shared future registration contract with the Process Broker once it exists. This
is deliberately the smaller, more conservative move available at each decision point — every rejected
alternative (heartbeats, an idle-kill for `ShellNodeRunner`, code-level convergence) added a new mechanism or
new inferred-liveness heuristic; the accepted decisions instead all reuse a signal or a lifecycle primitive
that already exists somewhere in this codebase today.

---

## Key files

| File | Role |
|------|------|
| `agentmux-bashwrap/src/main.rs` | Two subcommands (`hook`, `exec`); confirms one-shot-per-tool-call lifecycle, not a daemon |
| `agentmux-bashwrap/src/hook.rs` | `PreToolUseInput` struct (§1.2) — the exact place `run_in_background` gets dropped |
| `agentmux-bashwrap/src/bash_wrap.rs` | `DEFAULT_IDLE_KILL_TIMEOUT` (600s, §1.3), kill-tree logic, idle/success race tests |
| `agentmux-bashwrap/src/wps_client.rs` | Bashwrap's only outbound integration — `tool_chunk` streaming, unrelated to any liveness/tracking mechanism |
| `agentmux-srv/src/backend/shell_node.rs` | `ShellNodeRunner` — the other, unrelated execution surface behind `mcp__agentmux__Shell`; no idle-kill |
| `frontend/app/view/agent/activity/{types,shell-adapter}.ts` | `ShellNode`/`PinnedActivity` — what the dock actually sees (only `shell_node.rs`-backed processes, never bashwrap-executed ones) |
| `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` | First occurrence of this report's core finding, arrived at independently ten days earlier |
| `docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md` | The pager-hang bug the idle-kill exists to guard against — legitimate original motivation, still valid for the foreground case |
| `docs/specs/SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20.md` | Documents both execution surfaces independently, from an unrelated bug (console-window flashes) |
| `frontend/app/view/agent/flows/launch-flow.ts` | Unrelated file, cited in §4 only as a third corroborating data point for "silently-absorbed failure states" as a recurring pattern |

---

## Appendix: research method

This report is grounded in two categories of evidence, kept explicitly separate throughout: (1) **live,
first-hand reproduction** during today's manual testing of PR #2304 (§3.1, §3.2) — not inferred from logs
after the fact, but directly observed via process-tree inspection, structured-log reads, and a controlled
retry that isolated the exact cause; and (2) **direct reads of this codebase's own prior-art trail** — the
three existing `REPORT_LONGRUNNING_*`/`REPORT_PROCESS_ARCHITECTURE_*` documents, the Agent1 retro, the
pager-hang retro, and the console-window-elimination spec, each read in full rather than summarized
secondhand, plus direct source reads of `agentmux-bashwrap`'s three source files and `shell_node.rs` to
confirm every structural claim (the missing `run_in_background` field, the zero-hits grep against the six
tracking mechanisms, the absence of any timeout logic in `shell_node.rs`) against actual code rather than
inference from documentation alone. No claim in §1–§3 is sourced from documentation without a corresponding
direct code read; §4 is deliberately scoped as a pointer to existing analysis rather than new research, since
redoing that work would duplicate reports that already did it well.
