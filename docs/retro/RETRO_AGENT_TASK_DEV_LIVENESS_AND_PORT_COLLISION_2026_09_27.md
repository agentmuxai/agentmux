# Retro: an agent could not get a visible, verified `task dev` window (2026-09-27)

**Status:** retro
**Trigger:** the operator asked for `task dev TITLE="Agent3"` so they could look
at a shipped UI change (tighter Sysinfo margins, PR #3937, already merged and
tests-verified before this attempt — this was a *visual* confirmation only).
**Outcome:** the code change was never in question. Getting a stable, titled,
visually-checkable window up took roughly an hour and was eventually abandoned.
**Scope note up front:** nothing here is a violation of the isolation invariants
in `docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` (I1–I6).
Every kill this session issued was scoped to a PID whose own `CommandLine` was
first confirmed to reference this agent's own workspace path; no other agent's
window, pipe, or data dir was ever touched. What follows is friction in the
*build/observe* path for an agent driving `task dev` through scripted,
non-interactive tool calls — a persona the existing multi-instance work
(written for a human at their own keyboard, or for the app's own runtime
lifecycle) doesn't fully cover.

---

## TL;DR

Four separate, independently-diagnosed problems, each already partly addressed
by prior work in this repo, none of them fully closed for the "agent scripting
`task dev` unattended, on a host with several other agents doing the same"
case:

1. **Windows `MAX_PATH` broke the native CEF build**, for the exact reason
   `docs/retro/RETRO_CEF_C1083_PARALLEL_BUILD_RACE_2026_07_14.md` already
   diagnosed and fixed with `subst`. That retro's fix is real, but nothing
   applies it automatically — every agent whose workspace path is long enough
   has to rediscover and manually re-apply it, every time.
2. **The per-clone Vite port (`docs/analysis/ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md`'s
   own suggested follow-up, since shipped as `AGENTMUX_VITE_PORT`) still
   collided in practice**, and its collision-recovery logic has two real gaps
   this session hit directly: a liveness check that can't see a half-dead
   process, and an ownership check that breaks across a `subst` drive-letter
   change of the *same* clone.
3. **A long-running, successful `task dev` gives no way to confirm it worked**
   from a script: Windows fully buffers redirected stdout for a non-console
   process, so a log file shows nothing — not on failure-in-progress, and
   (because a successful session never exits) *never* on success either. The
   only working signal is OS-level (process list, window title), and even
   that title update is gated on the frontend reaching a specific point in
   its own init sequence, which this session's stuck Vite instance never
   reached.
4. **Fully detaching `task dev` from the invoking shell didn't work** in this
   sandboxed environment; PowerShell's `Start-Process` accepted the launch
   but its child never executed the intended command. This one is reported
   without a confirmed root cause — see §5.

None of these needed a code fix to *this session's actual deliverable* (the
Sysinfo PR was already merged, tested, and typechecked before any of this
started) — they're squarely about the gap between "a human can run this app"
and "an agent can reliably launch, observe, and clean up this app from
scripts, alongside several peers doing the same on one shared host."

---

## Evidence

### 1. `MAX_PATH` (confirmed, matches the existing retro exactly)

Workspace: `C:\Users\user\.agentmux\agents\agent3-0630k\work\survey\agentmuxai__agentmux`
(78 characters before even reaching a repo-relative path). `cargo build --release -p agentmux-srv`
succeeded; the vendored `cef-dll-sys` build script's CMake/Ninja step failed
identically to the July retro:

```
libcef_dll\ctocpp\test\api_version_test_scoped_library_child_child_v1_ctocpp.cc : fatal error C1083: Cannot open compiler generated file: '': Invalid argument
ninja: build stopped: subcommand failed.
thread 'main' (...) panicked at .../cmake-0.1.58/src/lib.rs:1132:5
```

Applying that retro's documented, verified fix —
`cmd //c "subst K: <repo path>"`, then building from `K:\` — immediately
produced a clean build: `agentmux-cef.exe` and `agentmux-srv.exe` both
compiled, zero `C1083` occurrences.

**What's missing isn't the fix — it's that nothing applies it.** The July
retro's own "Root fix for the repo itself (not attempted here)" section
already named this: agent workspace paths are provisioned long and deeply
nested by convention (`~/.agentmux/agents/<agent-id>/work/<...>/<owner>__<repo>`),
and every agent whose path crosses the threshold re-discovers the exact same
failure independently. This is the second time it's been diagnosed from
scratch in this repo's own retro history (that we know of).

### 2. Vite port collision (new findings, root-caused against the actual Taskfile logic)

`Taskfile.yml`'s `dev` task derives a port deterministically:

```sh
PORT_OFFSET=$(( $(echo -n "$CLONE_ROOT" | cksum | cut -d' ' -f1) % 200 ))
AGENTMUX_VITE_PORT=$((5173 + PORT_OFFSET))
```

— a 200-slot hash of the absolute workspace path, exactly the follow-up the
May analysis flagged as "not required for isolation... two-line change." It
shipped. It is not collision-*proof*, only collision-*reduced*: with several
agents' clones on one host (this session observed windows titled `Lark` and
`Korp`, i.e. at least two other live dev sessions, likely more given the
process count), 200 buckets is not a lot of headroom, and this session's own
first run landed on port 5298 — already in use.

The Taskfile *does* have a designed-for-exactly-this recovery path: before
spawning Vite, it curls the candidate port, and if something answers, tries to
verify the answering process is *this clone's own* orphaned Vite (by matching
the process's `CommandLine` against `CLONE_ROOT`) and reap it — refusing to
touch a live *foreign* clone's Vite. Two things about that path did not hold
up under this session's actual sequence of events:

- **The liveness check is an HTTP curl, not a socket/PID check.** A Vite
  process that is alive, has the port bound, but is stuck in its own
  restart-loop (see the next section) may not answer an HTTP GET within the
  1-second timeout. The pre-flight `curl -s --max-time 1 "$VITE_URL"` then
  reports "not busy," `task dev` proceeds to spawn a fresh Vite anyway, and
  *that* Vite hits the real OS-level bind failure directly:
  `Error: Port 5999 is already in use`. The port-busy branch's reaping logic
  never runs at all in this case, because the pre-check that gates it already
  returned the wrong answer.
- **The ownership match is a path-string comparison, and `subst` changes the
  string.** After applying the `MAX_PATH` fix above, this session's
  `CLONE_ROOT` was `K:\...`; an orphaned Vite from an *earlier* attempt (run
  before the `subst`, or with a manually-overridden port) had a `CommandLine`
  referencing the original `C:\Users\...` path. The ownership check
  (`grep -qF "${our_root}/"`) would correctly refuse to reap that process —
  it looks foreign — even though it was, in fact, this exact same agent's own
  earlier orphan. This is a real, if narrow, interaction between the two
  documented workarounds: applying the `MAX_PATH` fix makes the Vite
  port-reaping fix less reliable for *that specific clone*, for the rest of
  that shell's lifetime.

Net effect observed: repeated `Error: Port <n> is already in use` /
`vite.config.ts changed, restarting server...` cycles that did not
self-resolve, across three different chosen ports, until every stray process
was hunted down and killed by hand.

### 3. No liveness signal from a script (new finding)

Every attempt to `tail` a redirected log file — whether the harness's own
background-task capture, or an explicit `> file.log 2>&1` this session
controlled directly — showed **nothing** while `task dev` was actually
building or serving successfully, and (for the two runs that failed) dumped
its *entire* multi-thousand-line output only at the moment the process
exited. **Correction (Codex review on the PR for this retro): the original
draft attributed this to "standard Windows CRT full-buffering," stated as a
blanket rule.** That doesn't hold up: a fully-buffered stream still flushes
whenever its buffer fills, not only at exit, so many minutes of continuous
`cargo`/`ninja` compiler output — easily well past any typical few-KB buffer
— should have produced *some* incremental lines long before exit, and
`task dev`'s output is a mix of Go (`task` itself), shell, Rust, and Node
processes, not one CRT stream to begin with. Something in this pipeline did
suppress nearly all incremental output — plausibly full buffering somewhere
in that chain, or an aggregation layer in the harness's own capture (the
`task`/`agentmux-bashwrap` wrapping this session's tool calls go through) —
but which one, and why literally nothing flushed across 15+ minutes of real
build output in one run, was **not conclusively isolated here**. Treat the
mechanism as unresolved; the *practical* conclusion below (poll OS state, not
log content) held regardless of which mechanism turns out to be responsible:

- A **failed** run's diagnostics are only visible after the fact (fine — you
  get them all at once).
- A **successful, long-running** run's stdout is *never* visible this way,
  because the process that would eventually flush it never exits. The only
  way this session could confirm real progress was polling `tasklist`,
  `Get-Process -Id ... | Select MainWindowTitle`, and `Get-NetTCPConnection`
  directly — OS state, not the app's own output.
- Confirming *readiness*, not just *aliveness*, was harder still: the window
  exists and shows `Responding: True` from the moment the CEF process starts,
  long before the frontend has actually connected to anything. The custom
  window title (`frontend/app-init.ts`'s `VITE_DEV_TITLE` application, gated
  on `await initMuxWrap(initOpts)` completing) is actually a decent proxy for
  "fully up," precisely *because* it's one of the last things `app-init.ts`
  does — but nothing documents that, so this session spent real time confused
  about why the window stayed titled plain `AgentMux` (the answer, in
  hindsight: the frontend was stuck behind the Vite restart-loop in §2 the
  entire time, and never reached that line of `app-init.ts` at all).

### 4. Detached launch didn't execute (reported, not fully diagnosed)

To get a `task dev` session that would survive independently of this agent's
own tool-call boundaries, this session tried
`Start-Process -FilePath bash.exe -ArgumentList @('-lc', $cmd) -RedirectStandardOutput ...`.
`Start-Process` returned successfully and spawned `bash.exe` processes at the
expected time, but neither the redirected log file nor a subsequent `task.exe`
process ever appeared — the intended command did not appear to run at all,
including for a trivial `echo hello > file` smoke test using the identical
mechanism. Plausible causes not distinguished here: a different PATH/profile
for a process whose parent is `powershell.exe` rather than this session's own
`agentmux-bashwrap.exe`; the sandbox specifically constraining what a
detached, non-supervised process can do; or a `-ArgumentList` quoting issue
specific to this combination. **Flagged as an open question, not a finding** —
worth a follow-up investigation with more controlled, incremental tests than
this session had budget for.

(Separately: the tool-call boundary itself was *not* the problem it first
looked like. An earlier interruption this session initially suspected was the
harness killing a backgrounded job turned out, on review, to be this session's
own explicit `taskkill /T /F` against that process's PID, run for an unrelated
cleanup reason a few tool calls later. Plain `run_in_background` jobs survived
many subsequent unrelated tool calls without incident elsewhere in this same
session — the false lead cost some time and is worth naming so a future
retro doesn't repeat the misdiagnosis.)

---

## Why this wasn't caught by existing docs

- `RETRO_CEF_C1083_PARALLEL_BUILD_RACE_2026_07_14.md` diagnosed and fixed §1
  once already, but scoped its fix to "apply `subst` yourself" rather than
  "make new agent workspaces short enough that this doesn't recur," so it
  recurs.
- `ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md` explicitly scoped
  the Vite port out of its isolation guarantee ("already loud, not silent
  cross-contamination... two-line change but not required") — true as far as
  it goes, but the two-line change that shipped is a probabilistic hash, not
  a real allocator, and its own recovery path has the two gaps in §2 above.
- `SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md`'s I1–I6 invariants
  are specifically about resources the **app's own runtime code** claims
  (pipes, job objects, data dirs) between distinct **running instances**.
  They say nothing about an **agent's own ad hoc diagnostic commands**
  (`taskkill`, `Stop-Process` with a loose filter) during a build/troubleshoot
  session, which is a different risk entirely: the invariants protect against
  AgentMux's code misbehaving, not against an operator/agent's own overly
  broad cleanup command taking out a neighbor's window. This session's
  cleanup queries did, at one point, return process lists mixing this
  session's own strays with what looked like other agents' processes before
  being narrowed to exact `CommandLine` matches before anything was killed —
  worth naming as a near-miss, not an incident.
- None of the docs above are written with "an agent is driving this
  non-interactively, via scripted tool calls, and needs a machine-checkable
  success signal" as the reader. All of them assume a human watching a
  terminal or a window, for whom "Vite ready" scrolling past, or a window
  visibly appearing, *is* the liveness signal.

---

## Plan: what would actually iron this out

Ordered by leverage (how many future agent sessions it saves) versus cost.

### P1 — A project `run` skill for AgentMux itself (cheap, highest leverage)

No project-specific skill for "launch and drive this app" exists yet (checked
at the start of this session: no `.claude/skills/*/SKILL.md` in this repo).
Every agent that needs to smoke-test a UI change re-derives this entire
workflow — and its pitfalls — from scratch, as this session just did over
roughly an hour. Write one, covering:

- Check `pwd -P` length before building; if the workspace path is long,
  `subst` a short drive letter *first*, as a documented first step, not a
  panic-diagnosed fallback.
- Don't trust the auto-derived `AGENTMUX_VITE_PORT`. Probe candidate ports
  directly (a real socket-connect probe, not relying on Taskfile's own
  curl-based check) and pass an explicit, confirmed-free
  `AGENTMUX_VITE_PORT=<n>` up front.
- **How to confirm success without reading process output**: poll
  `Get-Process | Where MainWindowTitle -like "$expectedTitle*"`, not log
  tailing. **Correction (Codex): don't match by exact equality.** The `TITLE`
  value becomes the window *display name*, not the whole title; the actual
  OS title is built by `frontend/util/window-title.ts`'s `formatWindowTitle()`
  as `"<displayName> - <tabName> - AgentMux"` (or `"<displayName> - AgentMux"`
  with no tab — see `window-title.test.ts`, and the real examples this
  session observed, `"Lark - Tab 1 - AgentMux"` / `"Korp - Tab 1 - AgentMux"`).
  Expect a delay between "window exists" and "title is set" — that gap means
  the frontend hasn't finished `initMuxWrap` yet, not that anything is wrong.
- A precise, PID-scoped cleanup snippet: always resolve and print the exact
  `CommandLine` of anything you're about to kill and confirm it references
  your own workspace path, before killing it. Never filter by a loose pattern
  like `*cef-dev*` alone across a shared host.
- Note the buffering caveat explicitly, so the next agent doesn't spend time
  debugging "why is my log file empty" the way this session did.

This is the single highest-leverage fix here: it doesn't require touching
`Taskfile.yml` or any Rust/TS code, and it converts roughly an hour of
rediscovery into a five-minute, known-good procedure for every future agent.

### P2 — Make a real port allocator, not a bigger hash (moderate cost, closes §2 for good)

Replace the 200-slot hash with an actual "find a free port" step:

1. Try the hash-derived port first (keeps today's determinism/debuggability
   for the common single-agent case).
2. If a real bind (not a curl GET) fails, walk forward (`+1`, bounded retry)
   until a genuinely free port is found, print which one, and export it —
   rather than failing and asking the human/agent to pick one by hand.
3. Fix the two specific gaps found here regardless of (1)/(2): the pre-check
   should attempt an actual TCP connect/bind, not an HTTP GET (a hung Vite
   with the socket open but not answering HTTP is exactly the case that
   slipped through); and the ownership check should compare *canonicalized*
   paths (resolving a `subst` drive back to its real target, e.g. via
   `subst` output or `GetFinalPathNameByHandle`) rather than raw path
   strings, so a clone doesn't stop being reapable just because it changed
   drive letters mid-session.

### P3 — Shorten agent workspace provisioning (higher cost, closes §1 at the root)

This is the "root fix" the July retro already named and didn't attempt:
provision agent working directories under something shorter than
`~/.agentmux/agents/<agent-id>/work/<...>/<owner>__<repo>` — even
`C:\am\<short-hash>\` would very likely keep every real workspace under
`MAX_PATH` without needing `subst` at all. This is an AgentMux-host-tooling
change (outside this repo), not something a single agent session can land,
but it's the fix that makes P1's "check the path length first" step
unnecessary rather than merely automated.

### P4 — Investigate the detached-launch failure (lower priority, unresolved)

Worth a dedicated, narrower follow-up: does `Start-Process` from this
sandboxed shell actually execute its child at all, and if not, why (PATH?
profile? an active restriction?). Until answered, the practical guidance for
P1's skill is: don't rely on `Start-Process`-style detachment; a plain
`run_in_background` Bash tool call, left alone (no later `taskkill`/
`Stop-Process` against it) survived this session's own subsequent tool calls
without incident, and is the safer known-working pattern today.

---

## Lessons

- **Read the repo's own retros before troubleshooting from scratch.** Two of
  this session's four findings (`MAX_PATH`, the Vite port existing as a
  documented, deliberate non-goal) had prior, citable analysis; finding them
  *first* would have saved real time. They were found here, but only after
  a lot of blind troubleshooting first.
- **Don't attribute a process death to "the environment" before checking your
  own last few actions.** The `^C`/interrupted-job scare in this session was
  self-inflicted (an explicit `taskkill`) and cost time chasing a phantom
  root cause for the wrong problem.
- **A script needs a different liveness signal than a human does.** "Vite
  ready" scrolling past a terminal, or a window visibly popping up, means
  nothing to code polling a redirected file. Any future automation here
  should key off OS-observable state (window title, listening socket,
  process tree), never log content, for exactly the reason this session
  learned the hard way.
- **This was dev-workflow friction, not a product isolation bug.** Nothing
  here contradicts the "AgentMux runs multiple instances simultaneously"
  design intent or the I1–I6 invariants — those govern what the *app* claims
  automatically, and they held. What's missing is tooling for the case this
  design intent is explicitly meant to support: several agents each running
  their own `task dev` on one shared host at the same time, and one of them
  needing a machine-checkable way to know its own instance actually came up.

## References

- `docs/retro/RETRO_CEF_C1083_PARALLEL_BUILD_RACE_2026_07_14.md` — the
  `MAX_PATH` root cause and the `subst` fix this session re-applied.
- `docs/analysis/ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md` — the
  `clone_id` isolation work, and the Vite-port follow-up that shipped as
  `AGENTMUX_VITE_PORT`.
- `docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` — the
  I1–I6 invariant contract; scope note above explains why this retro doesn't
  claim a violation of it.
- `docs/analysis/ANALYSIS_MULTI_AGENT_SESSION_AND_WORKDIR_ISOLATION_2026-07-29.md`
  and `docs/retro/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`
  — a different, already-analysed multi-agent collision class (shared working
  directories, session/turn ownership); not what this session hit, but the
  same "multi-agent on one host" territory.
- `Taskfile.yml` (`dev` task, Vite port derivation and reaping logic) —
  exact lines quoted in §2 above.
- `frontend/app-init.ts` — `VITE_DEV_TITLE` application point referenced in
  §3.
- `.claude/skills/run/` (does not yet exist) — the gap P1 proposes to close.
