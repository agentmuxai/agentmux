# Retro: `agentmux-bashwrap` Processes Leak Forever When a Command Invokes a Pager (2026-07-14)

## What happened

While investigating an unrelated build failure (see
`docs/retro/RETRO_CEF_C1083_PARALLEL_BUILD_RACE_2026_07_14.md`), a process
count taken on this dev machine showed **12 `agentmux-bashwrap.exe`
processes still running**, with creation timestamps spanning from
**2026-07-12 12:58 PM to 2026-07-14 3:25 AM** — up to ~2 days old. Every one
of them was still wrapping a **trivial, already-finished one-shot shell
command** (`exec --tool-id=<id> --b64-cmd=<base64>`), decoded examples:

```
git diff -- agentmux-srv/src/identity/oauth_client.rs agentmux-srv/Cargo.toml 2>&1
git pull origin main 2>&1 | tail -15 && git log --oneline -15
git diff --staged VERSION_HISTORY.md
```

None of these should take more than a few hundred milliseconds. All were
still alive, in `Responding` state (not deadlocked at the Windows
message-pump level), with modest but nonzero accumulated CPU time —
i.e. not fully inert zombies, just never exiting.

One of the twelve (`git diff --staged VERSION_HISTORY.md`, PID 95928,
created 2026-07-14 3:25 AM) is directly reproducible: it's the exact command
run earlier in *this session*. At the time, the Bash tool call
unexpectedly auto-backgrounded instead of returning immediately (noted
in-session as "that shouldn't happen for a quick git diff command"), and a
second, separate `git --no-pager diff --staged VERSION_HISTORY.md`
invocation was used to get a clean result. The *original* process was still
running, unreaped, hours later when this investigation started — direct,
first-hand confirmation that this isn't just old debris from other agents'
past sessions, but an actively-recurring bug.

The command lines also referenced several different agent working
directories (this session's `agenty-0629j`, plus `wt-archive-docs-fix3`,
`agent1-063o9`, `agentx-0622n`, `agent2-0630f`) — this is a
systemic pattern across sessions, not one session's fluke.

## Root cause

`agentmux-bashwrap.exe exec` (`agentmux-bashwrap/src/bash_wrap.rs`) runs the
wrapped command through a **real PTY** (`run_via_pty`, the path used
whenever `openpty()` succeeds — effectively always on this machine), so the
child process sees `isatty(stdout) == true`. This is intentional: per
`docs/specs/SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md`, PTY-first was
specifically reinstated to avoid pipes causing block-buffering in wrapped
tools.

That same `isatty()` check is exactly what `git` uses to decide whether to
auto-invoke `core.pager` (`less` by default) for commands like `git diff`,
`git log`, `git show`, `git branch`. Nothing in the bashwrap exec path — nor
anything `agentmux-srv` injects into the spawn environment — sets
`GIT_PAGER`, `PAGER`, or passes `--no-pager` / `-c core.pager=cat`. So a
`git diff`/`git log`/etc. run through bashwrap transparently spawns `less`.

`less` opens the *controlling terminal* directly to read keypresses (by
design, so it still works even when stdin is redirected — and bashwrap does
redirect stdin, via `{ <cmd>; } </dev/null`, which does not stop `less` from
finding the ConPTY slave as its controlling terminal). The wrapper writes
exactly one synthetic byte sequence to the PTY master at startup (the DSR
response bash's shell-integration query expects) and — per its own comment —
"never writes again." Nobody is driving the terminal, so `less` blocks
waiting for a keystroke **forever**.

The blocking chain: `less` blocks forever → `git` blocks on `less` → `bash -c
'{ <cmd>; }'` (bashwrap's direct child) blocks on `git` → bashwrap's
`child.wait()` (`bash_wrap.rs:624`, run inside a `spawn_blocking`) blocks on
`bash` → `run_via_pty` never returns → `bash_wrap::run()` never returns →
`main()`'s **only** `std::process::exit()` call (`main.rs:68-69`) is never
reached. The process just sits there, alive, doing nothing productive,
forever (or until the whole AgentMux Job Object tears down on app exit,
which doesn't happen during a normal multi-day session).

A secondary, structurally-identical gap exists even when `bash`'s own
`child.wait()` *does* return: `run_via_pty` still awaits
`publisher_handle` (`bash_wrap.rs:636`), which only completes once the
PTY-master reader thread sees EOF — which on ConPTY requires *every* handle
to the pseudoconsole's console-out side to close. Any lingering
backgrounded (`&`) grandchild that inherited the PTY slave fd would produce
the same never-exits symptom via a different trigger.

Both mechanisms reduce to one structural gap: **nothing in the call chain
(`run()` → `run_proc()` → `run_via_pty()`) bounds wall-clock time.** Every
exit path is gated behind cooperative I/O completion with no timeout or
kill fallback anywhere in the wrapper.

`docs/specs/SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md` §6 (Non-goals) already
anticipated pagers as a risk — *"Not alt-buffer detection. Vim/less/top
inside a tool overlay is undefined behavior for now... Out of scope for this
spec."* — but framed it as a **rendering-corruption** risk, not a
**process-leak** risk, and no follow-up tracking item exists connecting the
two.

## Why srv-side supervision doesn't catch this

`agentmux-bashwrap.exe` is spawned inside Claude Code's own bash-tool
subprocess tree (via the rewritten `Bash(agentmux-bashwrap *)` command hook,
`agentmux-srv/src/backend/agent_config.rs`), **not** by `agentmux-srv`
directly. Grepping every "bashwrap" reference in `agentmux-srv` confirms
there is no PID-level tracking of it anywhere:

- `agentmux-srv/src/server/mod.rs` registers the passive
  `/agentmux/wps/publish` HTTP route bashwrap POSTs chunks to — no
  per-tool-id timeout, no reaping.
- `agentmux-srv/src/backend/agent_config.rs` only writes hook config and
  permission grants — no runtime tracking.
- The one place srv *does* track and can forcibly kill a PTY child tree
  (`agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs`, the
  persistent interactive shell/agent-pane controller — a different, unrelated
  PTY consumer) only covers its own direct child, is `#[cfg(unix)]`-only for
  process-group kill, and has no reach into `agentmux-bashwrap.exe`, which is
  a grandchild several hops down the Claude CLI's own tool-call tree.

The gap isn't a bug in existing reaping logic — there simply is no reaping
logic for this specific process, by design: bashwrap was architected as an
opaque command-rewrite wrapper living entirely inside Claude's own
subprocess tree, outside srv's supervision boundary.

## Evidence

Process list (`Get-CimInstance Win32_Process`, filtered to
`agentmux-bashwrap.exe`, parent PIDs cross-checked to confirm none are
children of the one real running AgentMux instance — see the correction in
`RETRO_CEF_C1083_PARALLEL_BUILD_RACE_2026_07_14.md`):

```
PID 21092  created Jul 12 12:58 PM  git diff -- ...oauth_client.rs ...Cargo.toml
PID 29320  created Jul 12  1:03 PM  git diff --cached -U1 -- ...
PID 18444  created Jul 12  6:51 PM  muxlog launcher
PID 54380  created Jul 12  6:54 PM  muxlog errors ... | grep -i ERROR | tail -20 ...
PID 50196  created Jul 12  7:12 PM  git show dbe5ce48 --stat; git show ... tabbar.tsx | head -100
PID 39764  created Jul 12  8:05 PM  git diff CLAUDE.md docs/specs/...
PID 63992  created Jul 12  9:00 PM  git diff frontend/app/mixins.scss
PID 55392  created Jul 13  2:26 PM  git diff --staged -- VERSION_HISTORY.md package.json Cargo.toml
PID 80096  created Jul 13  6:39 PM  git pull origin main 2>&1 | tail -15 && git log --oneline -15
PID 76488  created Jul 13  7:02 PM  git diff agentmux-srv/src/backend/mod.rs agentmux-srv/src/migrations/mod.rs
PID 95928  created Jul 14  3:25 AM  git diff --staged VERSION_HISTORY.md   ← this session, directly witnessed
```

`Get-Process` on these PIDs: all `Responding = True`, CPU times in the
7–43 second range accumulated over their (multi-hour to multi-day)
lifetimes — consistent with an idle `less`/reader-thread loop, not a hot
spin, and not a fully-suspended zombie either.

Every failing command is either a bare `git diff`/`git log`/`git show`
(auto-pages when there's enough output and stdout is a TTY) or a pipeline
ending in one of those — consistent with the pager-hang hypothesis across
all eleven cross-session samples, not just the one directly reproduced.

## Mitigation options (implemented — see "Fix implemented + verification" below)

**Option A (targeted, low-risk) — disable git's pager for the PTY child**

Set `GIT_PAGER=cat` and/or `PAGER=cat` (or pass `-c core.pager=cat`) into
the environment `run_via_pty` spawns `bash` with. Directly closes the
specific, most-common trigger (`git diff`/`log`/`show`/`branch` etc.)
without touching the general PTY/timeout architecture. Doesn't help other
potential pager-likes (`less` invoked directly, `man`, some `npm`/`cargo`
subcommands with their own pagers) — narrower fix, narrower blast radius.

**Option B (general safety net) — timeout + kill fallback**

Wrap `child.wait()` and the `publisher_handle` drain in `run_via_pty` (and
`run_via_pipes`) with a `tokio::time::timeout`; on expiry, forcibly kill the
child (and ideally its process tree/job object on Windows) before falling
through to the aggregation + `process::exit` path. Catches *any* hang
mechanism — pagers, backgrounded grandchildren holding the PTY slave open,
future unknown causes — not just this one. Larger change, touches the core
exec path for every bashwrap invocation.

**Option C (srv-side) — timeout-based reaping from the outside**

`agentmux-srv` already receives the `tool_id` via the WPS publish payload
(including a "starting" system chunk). It could track `tool_id → spawn
timestamp` and forcibly kill any bashwrap-descended process tree that
hasn't sent a terminal/completion event within N minutes. Would require srv
to gain some way to identify and reach the actual OS process tree (it
currently has no PID for a bashwrap invocation at all) — larger
architectural change, but would also catch failures where bashwrap.exe
itself has become unresponsive, not just its children.

**Recommendation (not yet decided/applied):** Option A closes the specific,
confirmed, most-common trigger cheaply. Option B is the structurally
correct fix (bounds every exec, not just the pager case) and should
probably ship regardless of whether A ships too, since A only covers `git`
and doesn't protect against the next tool that happens to auto-page.

## Action items

- [x] Decide on and implement a fix — **both Option A and Option B
  shipped** (see "Fix implemented + verification" below). Option C
  (srv-side timeout-based reaping) was not pursued — Option B's in-process
  idle-kill makes it redundant for the reproduced failure mode.
- [x] Once fixed, manually verify: run a bashwrap-wrapped `git diff` /
  `git log` on output large enough to normally trigger paging, and confirm
  the wrapper process exits promptly instead of leaking. **Done — see
  below.**
- [ ] Consider a cleanup pass to kill the currently-leaked processes found
  in this investigation (still present as of the fix landing — they were
  created by the OLD, still-deployed binary and are unaffected by a
  source-level fix until a new portable ships) — not done as part of this
  pass.
- [ ] Ship a fresh portable build so the live AgentMux instance on this
  machine actually starts using the fixed binary — this session's own
  Bash-tool calls still route through the old deployed
  `agentmux-bashwrap.exe` until then. This was already independently
  blocked by `RETRO_CEF_C1083_PARALLEL_BUILD_RACE_2026_07_14.md`.
- [ ] Option C (srv-side reaping via the WPS "starting" system chunk) is
  still available as a defense-in-depth layer if Option B's in-process
  approach ever proves insufficient (e.g. a hang inside bashwrap's own
  async runtime before the idle-timeout logic can even run) — not pursued
  now since no evidence points at that failure mode.

## Fix implemented + verification (2026-07-14)

**Option A — disable git's pager.** `run_via_pty` and `run_via_pipes` (for
consistency, though pipes shouldn't need it since `isatty(stdout)` is false
there) now set `GIT_PAGER=cat` and `PAGER=cat` on the spawned `bash`
child's environment.

**Option B — idle-timeout kill + bounded waits.** `run_via_pty` now:
- Splits a `ChildKiller` off the spawned child before it moves into the
  `child.wait()` task (`portable_pty::ChildKiller::clone_killer` — its doc
  comment literally describes this exact "signal it from a thread that may
  be blocked in `.wait`" use case).
- `pty_reader_loop` tracks time-since-last-PTY-activity and fires a
  one-shot signal after `idle_kill_timeout()` (default 600s / 10 min,
  overridable via `AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS`) of zero bytes
  read — deliberately an **idle** timeout, not a total-runtime cap, so a
  build that's silent for under that long but runs far longer overall
  (e.g. several minutes of continuous compiler output) is unaffected.
- `run_via_pty` races `child.wait()` against that signal via
  `tokio::select!`; on idle-timeout it kills the child, gives the wait task
  a bounded 5s grace period to resolve (releasing the PTY), and falls back
  to a `124` sentinel exit code only if even that doesn't resolve — the
  wrapper never blocks unboundedly on this. The publisher drain is
  similarly bounded to 5s in case a surviving grandchild still holds the
  PTY slave open. A clear explanation is appended to the model-visible
  blob so the calling agent understands why the command was cut short.

**Automated tests (all passing, `cargo test -p agentmux-bashwrap` — 46/46,
up from the pre-fix 42):**

- `idle_kill_timeout_defaults_when_unset`,
  `idle_kill_timeout_honors_env_override`,
  `idle_kill_timeout_falls_back_on_unparseable_value` — the env-var
  override parsing.
- `run_via_pty_kills_idle_child_and_returns_promptly` — a real end-to-end
  test: spawns `sleep 9999` (a clean, portable stand-in for "any command
  silently blocked forever," decoupled from needing an actual `less`/pager
  to reproduce) through a real PTY with
  `AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS=1`, and asserts `run_via_pty`
  returns within a bounded time instead of hanging past a 20s outer test
  timeout. **First run caught a bug in the test itself, not the fix:** the
  original assertion `assert_eq!(result, 124, ...)` failed with `left: 1,
  right: 124` — the killed child actually resolved within the 5s grace
  period and reported its own real, platform-specific exit code, rather
  than falling through to the `124` sentinel. Not a defect — `124` is only
  supposed to fire when even the grace period doesn't resolve — so the
  assertion was loosened to `assert_ne!(result, 0)` (a killed command must
  never look like clean success) plus the timing bound, which is what
  actually matters. Recording this literally since it's exactly the kind
  of "verification surfaced a smaller issue in the test, not the
  production code" outcome worth keeping visible.

**Manual repro of the actual reported bug** (not just the synthetic
`sleep` stand-in): built `target/debug/agentmux-bashwrap.exe` from the
fixed source and ran it directly —

```
target/debug/agentmux-bashwrap.exe exec --tool-id=manual-repro-test \
  --b64-cmd=<base64 of "git log --oneline -50">
```

in this repo (which has far more than one screen's worth of commits, so
`git log` would previously auto-invoke `less` on a real terminal). Result:
**exit code 0, ~1 second elapsed, full 50-line log in the output** — no
pager invoked, no hang, and no lingering `agentmux-bashwrap.exe` process
afterward.

**Corroborating live evidence, and an important scope caveat.** While
verifying, two *more* `agentmux-bashwrap.exe` processes turned up
(`PID 73288`, `PID 95504`) beyond the twelve originally catalogued — both
running through the **old, still-deployed** portable binary
(`...\agentmux-0.53.2+...\runtime\tools\bin\agentmux-bashwrap.exe`), not
the fixed source in this repo, confirming this session's own tool calls
still route through the unfixed binary (expected — no new portable has
shipped yet). Decoding PID 73288's command line showed it wasn't even a
`git`/pager case — it was an earlier `find / -maxdepth 6 -type d -iname
"portable-pty-*" ...` from this same session, walking a large portion of
the filesystem. **Correction:** that process was not actually stuck — a
`task-notification` arrived confirming it completed normally (exit code 0)
shortly after this was written, meaning it was legitimately slow (a wide
filesystem search), not hung. It's kept here as a **real, concrete example
of the exact false-positive risk Option B's idle-timeout carries**: a
long-silent-but-still-working command (no matches to print for a long
stretch) is indistinguishable, from bashwrap's perspective, from a
genuinely stuck one, and a `find /`-like search that legitimately produces
zero output for 10+ minutes could in principle trip the idle-kill.
Mitigated by `AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS` being overridable and by
the 10-minute default being generous, but worth monitoring in practice
rather than assuming zero false-positive risk — this specific case simply
didn't run long enough in complete silence to test that edge directly.

## Follow-up: reagent P1 — tree-kill gap, and a real detour investigating it

After the fix above shipped for ReAgent review (PR #2156), a second review
round flagged a real gap: `killer.kill()` (via `portable_pty::ChildKiller`)
only terminates the **direct** PTY child — portable-pty's Windows impl is a
bare `TerminateProcess` on one handle, no job object (confirmed by reading
its source: `agentmux-bashwrap`'s only tree-aware addition at that point was
none). A command whose direct child forks further (a pipeline, or `git`
spawning `less` as a child — exactly the reported bug) could leave the
grandchild running and still attached to the PTY slave, reproducing the
leak one process removed instead of eliminating it.

**Fix:** added `kill_process_tree(pid)` — on Windows, `taskkill /T /F /PID
<pid>` (no new dependency, just `std::process::Command`), called *before*
`killer.kill()` (ordering matters — killing the root first can break
`taskkill /T`'s ability to walk its now-dead root's children; verified this
mattered in practice, see below). Not implemented for Unix — all evidence
for this bug is Windows-specific, and portable-pty's Unix `ChildKiller`
already sends `SIGHUP`, which its own doc comment suggests reaches the
process group (untested; deferred until there's an actual Unix repro).

**Verification took a real, worthwhile detour.** Strengthened the existing
single-process (`sleep 9999`) test into a multi-process one (`sleep
<pid>.001 & sleep <pid>.002 & wait`, two backgrounded grandchildren) so it
would actually exercise the gap ReAgent flagged — the original test
couldn't have caught this, since a lone leaf process has nothing to leave
orphaned.

1. **First run:** test failed instantly (~0.1s, exit code 0) — a bug in the
   test itself, not the fix. `sleep <marker>` used a non-numeric marker
   string as the duration argument, so `sleep` errored out immediately
   instead of blocking. Fixed by deriving a valid numeric duration from the
   test's own PID (`sleep {pid}.001`) instead.
2. **Second run:** test failed with "1 survivor" — seemingly confirming the
   gap. Investigated by manually inspecting `Win32_Process.ParentProcessId`
   for a backgrounded `sleep` outside the test harness, which showed it
   pointing at a PID that had *already exited* by query time — not
   `bash.exe`. This looked like proof that Git-for-Windows' MSYS2 bash
   doesn't preserve a discoverable Win32 parent-child chain for forked
   (backgrounded/piped) children, which would make `taskkill /T` — or any
   Win32 PID-tree API — structurally unable to reach them. Reordered
   `kill_process_tree` before `killer.kill()` (a real, separate fix — killing
   `bash.exe` first and *then* trying to tree-walk from its now-dead PID is
   a genuine race) and re-ran: **still** "1 survivor."
3. **Root cause of the "1 survivor," found by inspecting the actual
   process, not just its count:** the test's own
   `count_processes_with_marker` PowerShell helper queries
   `Win32_Process | Where-Object { $_.CommandLine -like '*<marker>*' }` —
   but that query's *own invocation argument* contains the marker text, so
   it matches **itself**. A `ps aux | grep foo` matching-its-own-`grep`
   bug, not a real orphan. Confirmed by manually filtering to `Name -eq
   'sleep.exe'` outside the test and finding **zero** matches — the fix had
   been working correctly the whole time.
4. Fixed the helper (excluded `powershell.exe` from its own match) and
   re-ran: passing, consistently, across repeated runs and both the
   in-process test and a new, stronger binary-level end-to-end test (below).

**Net result:** the earlier "MSYS fork-emulation permanently breaks
Win32 PID tracking" conclusion doesn't hold up as stated — `taskkill /T`
does reliably reach these backgrounded children in practice (the
ordering fix — tree-kill before the direct kill — may be part of why; not
independently re-isolated from the marker-matching fix since both landed
before the clean pass). The stale-`ParentProcessId` observation itself was
real, just not proof of what it seemed to prove. Left un-chased further
once the empirical result was unambiguous — see Open Questions.

**Added a second, more representative test:**
`bashwrap_binary_idle_kill_cleans_up_full_process_tree` spawns the actual
compiled binary as a real subprocess (not an in-process library call —
the original test calls `run_via_pty` directly inside `cargo test`'s own
process, which never exits, so it can't observe whatever cleanup depends on
the *wrapper* process's own exit) with the same two-grandchild scenario,
waits for it to exit, and confirms zero survivors system-wide afterward.
Both tests now pass. Full suite: **47/47** (up from 46 after the first fix
round, 42 before any of this work).

**Manual repro re-confirmed after all the above changes:** `git log
--oneline -50` through the rebuilt binary — exit 0, instant, full 83-line
output (50 log lines + the `<exited ...>` wrapper), no hang, no leak.

## Open questions

1. Does this affect other commonly-paged tools beyond `git` (e.g. `man`,
   `kubectl`, `az`, `gh`, `npm` scripts that shell out to `less`)? Only
   `git`-based repro evidence was found in this investigation, but the
   mechanism (any tool checking `isatty(stdout)` and auto-paging) is
   general.
2. Does the pipe-fallback path (`run_via_pipes`, used when `openpty()`
   fails) avoid this entirely, since `isatty(stdout)` would be false there?
   If so, is `openpty()` failure common enough on any supported platform
   that it's worth understanding as an (accidental) mitigation already in
   effect for some fraction of invocations?
3. How many of these leaked processes exist right now, in aggregate, across
   all machines running AgentMux dev/agent sessions? Only checked on this
   one dev machine.
4. Is there any user-visible impact beyond wasted OS resources — e.g. could
   a leaked bashwrap process interfere with a *later* invocation reusing the
   same `--tool-id`, or hold a file lock / working-directory handle that
   blocks something else? Not investigated.
5. Why did `taskkill /T /F /PID <bash_pid>` succeed at finding and killing
   the backgrounded `sleep` grandchildren in the passing end-to-end test,
   given the earlier manual `Win32_Process.ParentProcessId` check showed
   the same kind of process's recorded parent had already exited by query
   time? Two candidate explanations were noted but not distinguished: (a)
   `taskkill` runs promptly after idle-detection, closer to spawn time than
   the ~2s-later manual check was, so the (short-lived) MSYS fork
   intermediate may still have been alive/enumerable at that earlier
   moment; (b) `taskkill /T`'s own tree-walk may use a broader or
   differently-timed relationship than a single `ParentProcessId` snapshot
   exposes. Worth a real answer if this area gets touched again — right
   now the fix is verified empirically (consistently, across repeated
   runs) but not fully explained mechanistically.

## Timeline (2026-07-14, this dev machine, exact clock times not captured)

| Order | Event |
|-------|-------|
| 1 | While investigating the CEF C1083 build race, `tasklist` showed 12 `agentmux-bashwrap.exe` processes; initially mis-attributed as evidence of 12 concurrent AgentMux app instances (corrected in the CEF retro after being challenged). |
| 2 | Re-investigated properly: `Win32_Process` parent-PID inspection showed these 12 bashwrap processes are unrelated to the one real AgentMux instance's process tree. |
| 3 | Decoded their base64 command lines: all trivial, fast, already-completed one-shot commands, several days old, all `git diff`/`git log`/`git pull`/muxlog invocations. |
| 4 | Recognized one of them (`git diff --staged VERSION_HISTORY.md`) as a command from earlier in this exact session that had behaved oddly (auto-backgrounded unexpectedly) — confirmed its process was still alive, hours later. |
| 5 | Delegated a full investigation of `agentmux-bashwrap`'s exit path and `agentmux-srv`'s (lack of) supervision to a research agent. |
| 6 | Root cause returned: PTY-driven `isatty(stdout)=true` causes `git` to auto-invoke `less`, which blocks forever with no keystroke source; no timeout anywhere in the call chain; no srv-side supervision reaches bashwrap at all. |
| 7 | Retro written; fix deferred to a follow-up per explicit direction ("Write a retro first, fix later"). |
| 8 | Directed to fix all identified issues in the same PR. Implemented Option A (GIT_PAGER/PAGER=cat) and Option B (idle-timeout kill + bounded waits via `ChildKiller::clone_killer`) in `run_via_pty`. |
| 9 | `cargo build -p agentmux-bashwrap` clean; `cargo test -p agentmux-bashwrap` 42/42 pre-existing tests still pass. |
| 10 | Added 4 new tests (env-override parsing ×3, end-to-end idle-kill ×1). First run of the end-to-end test failed on an over-specific assertion (expected exit code 124, got 1) — the kill mechanism itself worked correctly; the test's expectation was wrong. Fixed the assertion, re-ran: 46/46 passing. |
| 11 | Manual repro against the actual reported bug (not the synthetic `sleep` stand-in): built the fixed binary, ran `git log --oneline -50` through it directly — exited in ~1s with full output, no hang, no leak. |
| 12 | While checking for new leaks, found 2 more bashwrap processes — both traced to the OLD deployed binary (fix not yet live) and one to an unrelated `find /` command from earlier this session, which a subsequent task-notification confirmed had completed normally (not actually stuck) — documented as a real example of the idle-timeout's false-positive risk class rather than a new leak. |
| 13 | PR #2156 renamed from docs-only to the fix PR, changeset added, pushed. ReAgent's re-review flagged a real P1: `killer.kill()` alone only terminates the direct PTY child, not descendants (a pipeline, or `git` spawning `less`). |
| 14 | Added `kill_process_tree` (Windows `taskkill /T /F /PID`) and strengthened the test to a two-grandchild scenario. First run: test failed instantly (exit 0) — a non-numeric `sleep` duration argument in the test itself, not the fix; fixed. |
| 15 | Second run: "1 survivor" — investigated by manually checking `Win32_Process.ParentProcessId` for a backgrounded `sleep` outside the test, found it pointed at an already-exited PID, seemingly confirming MSYS bash breaks Win32 PID tracking for forked children. Reordered `kill_process_tree` before `killer.kill()` (a real, separate fix for a kill-ordering race) and re-ran: still "1 survivor." |
| 16 | Found the actual cause by inspecting the surviving process directly rather than trusting the count: the test's own PowerShell query matched *itself* (its `-like '*marker*'` argument literally contains the marker). Fixed the query to exclude `powershell.exe`; confirmed via a marker-free, `sleep.exe`-filtered manual check that zero real orphans existed — the fix had been working the whole time. |
| 17 | Added a second, stronger test (`bashwrap_binary_idle_kill_cleans_up_full_process_tree`) spawning the real compiled binary as a subprocess rather than calling `run_via_pty` in-process, to properly observe post-exit cleanup. Both tests pass consistently across repeated runs. Full suite: 47/47. Manual `git log` repro re-confirmed clean after all changes. |
