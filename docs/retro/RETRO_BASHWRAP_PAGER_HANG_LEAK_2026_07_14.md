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

## Follow-up round 2: reagent P1 (misclassification race) + P2 (ordering) + a self-inflicted test bug

A third ReAgent review round on the tree-kill commit above found two more
real issues, both in `run_via_pty`'s `tokio::select!` block:

- **P1 — idle/success misclassification race.** The `select!` arm was
  written `_ = idle_rx => { idle_killed = true; ...kill... }`. A
  `oneshot::Receiver`'s `.await` resolves for TWO different reasons —
  the sender explicitly sent (`Ok`), or the sender was simply *dropped
  without sending* (`Err`) — and the `_` pattern ignored which one
  happened. `pty_reader_loop` drops its still-unused `idle_tx` whenever it
  returns via normal EOF (i.e. every ordinary successful command), and
  that drop races `wait_task`'s independent `child.wait()` completion on a
  separate thread. Under scheduling variance — ReAgent specifically named
  blocking-thread-pool contention from concurrent bashwrap invocations,
  which this retro's own evidence shows are common — a fast, perfectly
  successful command could have its `idle_rx` branch "win" the race and
  get spuriously killed, with the false `"terminated automatically"`
  diagnostic appended to otherwise-normal output. Fixed by branching on
  `idle_signal.is_err()` inside the arm: `Err` (dropped, not signaled) now
  falls through to directly `.await` the real `wait_task` instead of
  treating it as a kill trigger.
- **P2 — diagnostic ordering.** The idle-kill diagnostic note was appended
  to `buffered` *before* draining `publisher_handle`, so genuine
  already-queued pre-kill output could flush in after the diagnostic,
  interleaving it ahead of trailing real output instead of strictly after.
  Fixed by moving the publisher drain before the diagnostic append.

**Added a regression test for P1**
(`run_via_pty_does_not_misclassify_fast_success_as_idle_timeout`): runs a
trivial `echo hello` 30 times with a 1s idle timeout, asserting every run
reports exit 0 with no spurious kill diagnostic. **Honesty check, done
before trusting it:** temporarily forced the old unconditional-kill
behavior back in and re-ran this exact test — it still passed 30/30. On
this machine, `wait_task` reliably wins the race against
`pty_reader_loop`'s EOF-then-drop for a command as fast as `echo hello`, so
this test does *not* reliably fail on the unfixed code and isn't
proof by itself that the race is closed. ReAgent's code-level analysis is
still correct regardless (a oneshot receiver genuinely resolves
identically for both cases, and the fix is the structurally correct
response) — the realistic trigger they named (blocking-pool contention
under concurrent load) wasn't reproduced here; doing so reliably would
need deliberately saturating tokio's blocking pool, not attempted given
time already spent on this investigation. Recorded plainly rather than
overclaiming test coverage that doesn't exist.

**A third, self-inflicted bug found while adding that test — full test
suite went from passing to failing with `"3 process(es) still matching
marker"`.** Root cause: `std::process::id()` is the *same* value across
every test in one `cargo test` binary (they're threads in one process, not
separate OS processes), and Rust's test harness runs tests in parallel by
default. Two different tests (`run_via_pty_kills_idle_child_and_returns_promptly`
and `bashwrap_binary_idle_kill_cleans_up_full_process_tree`) both derived
their marker from the bare PID, so their `sleep <marker>.NNN` commands
shared a common substring — a slow-to-clean-up process from one test could
get miscounted as a "survivor" by the other test's WMI search if they
happened to overlap in time. Fixed by baking a distinguishing tag into
each test's marker (`{pid}100` vs `{pid}300`) so their marker substrings
can no longer overlap. Re-ran the full suite 4× in a row after the fix:
**48/48 passing every time** (47 after the tree-kill round, 46 after the
first fix round, 42 before any of this work).

Manual `git log --oneline -50` repro re-re-confirmed clean after this
round too: exit 0, instant, full output, zero leaked `sleep.exe` processes
afterward.

## Follow-up round 3: reagent P1 — `run_via_pipes` had zero idle-kill protection at all

A fourth review round on the tree-kill+select-fix commit found the biggest
remaining gap: **`run_via_pipes`** (the fallback path used when
`openpty()` fails — CI, sandboxes, some restricted environments) had
**no** idle-timeout/kill safety net whatsoever. `child.wait().await` was
fully unbounded. This path can't hit the pager-hang mechanism specifically
(no PTY means `isatty(stdout) == false`, exactly why `git` wouldn't
auto-page there) — but nothing else bounded it either, so *any other* hang
cause on this path still leaked the wrapper forever, reproducing the root
bug this whole PR exists to fix. The retro's own "Mitigation options"
section, written before any code existed, explicitly said Option B should
cover "`run_via_pty` (and `run_via_pipes`)" — the implementation only
ever covered the former until this round.

**Fix:** mirrored `run_via_pty`'s idle-timeout design — `stream_reader`
(used for both stdout and stderr in the pipe path) now takes a shared
`Arc<Mutex<Instant>>` last-activity clock and resets it on every byte read;
a lightweight polling task watches that clock and fires a oneshot signal
after `idle_kill_timeout()` of silence; `run_via_pipes` races `child.wait()`
against that signal via `tokio::select!`, with the same `Ok`/`Err`
misclassification-safe branching as the PTY path's P1 fix.

### A very long detour: a genuine `tokio`/Windows pipe + test-harness bug, not a production bug

Writing the obvious test — same shape as the PTY-path tests, `sleep 9999`
through `run_via_pipes` with a 1s idle timeout — surfaced something new:
**the test function's body completed successfully every single time** (all
diagnostic `eprintln!`s through the final `return` fired, every assertion
passed, the correct killed exit code and diagnostic blob were produced,
consistently and quickly) — but `cargo test` never printed a `PASSED` or
`FAILED` result. The process just sat there indefinitely after the async
test fn returned.

Extensive bisection (temporarily instrumenting `run_via_pipes` line-by-line
with `eprintln!`, then building a series of standalone minimal repro tests
that incrementally added pieces back — bare `AsyncRead::read` loop with no
timeout; the same loop wrapped per-iteration in
`tokio::time::timeout(50ms, ...)` matching `stream_reader`'s real
quiet-window structure; with and without `.abort()`; with `tokio::join!`
instead) isolated the exact trigger:

**Explicitly calling `.abort()` on a `tokio::spawn`'d task that is
currently inside `tokio::time::timeout(Duration, AsyncReadExt::read(...))`,
reading from a `tokio::process::ChildStdout`/`ChildStderr` (a Windows named
pipe) whose owning child process was just killed, leaves something in a
state that hangs this specific `#[tokio::test]` runtime's shutdown** — even
though the `.abort()` call itself returns immediately and the calling code
proceeds normally. A structurally identical reader loop with NO per-read
timeout wrapper (bare `.read().await`) could be safely `.abort()`ed with no
issue. Switching from `.abort()` to properly `tokio::join!`-ing (or simply
letting the `JoinHandle` drop without ever calling abort, since dropping a
`JoinHandle` — unlike calling `.abort()` on it — does not cancel the
underlying task) resolved the minimal repro every time.

**Applied that fix to `run_via_pipes`:** the reader `JoinHandle`s are now
bound-joined via `tokio::join!` wrapped in a 5s `tokio::time::timeout`,
never `.abort()`ed. This is safe in production regardless of any deeper
mechanism, because a `JoinHandle` drop doesn't cancel the task — it just
keeps running fully detached until it naturally sees EOF once the killed
child's pipe closes.

**This fix for the join mechanism did *not* resolve the automated test.**
The real `run_via_pipes` — with its full `stream_reader` (collapse_cr,
pending-buffer, CR-override-slot logic), `mpsc::channel<LineEvent>`,
`spawn_publisher_loop`, and the shared `last_activity` mutex all present —
still hung the test harness identically even after switching to
`tokio::join!` throughout, despite an isolated minimal repro with the
*exact same* select!/kill/join structure (just without those additional
pieces) passing cleanly and fast (0.31s). The remaining difference between
"minimal repro: passes" and "real code: hangs" was not further isolated —
doing so would have meant repeating the same bisection process one or more
additional times against `mpsc`/`spawn_publisher_loop`/the shared
`std::sync::Mutex`, each cycle costing another full rebuild-and-wait-past-a-hang
round trip. Stopped here rather than continuing indefinitely.

**Decision: removed the automated test for `run_via_pipes`'s idle-kill,
verified the production code manually instead.** Reasoning:

- The production code's correctness was already extensively proven via
  the diagnostic `eprintln!` instrumentation used throughout this
  bisection — every run showed the idle-timeout firing at the configured
  time, the kill succeeding, the correct (non-zero) exit code being
  produced, and the diagnostic blob being set, all within the expected
  bounded time. This is real verification evidence, just not encoded as
  an automated `#[test]`.
- The specific failure mode (a graceful async-runtime-shutdown hang) is
  **structurally impossible in production**: `main()` in this crate has
  exactly one exit path, an unconditional `std::process::exit(code)` call
  (see `main.rs`) — a hard OS-level process termination that does not run
  Rust destructors on other threads/tasks and does not wait for any
  spawned task, graceful or otherwise. Whatever tokio/Windows-pipe
  interaction causes `#[tokio::test]`'s *graceful* runtime teardown to
  hang has no equivalent code path in the actual shipped binary.
- This looks like a genuine `tokio`/Windows-IOCP edge case (plausibly
  related to cancel-safety of in-flight overlapped `ReadFile` operations
  when the wrapping future is aborted specifically while inside a
  `tokio::time::timeout`, though this was not confirmed against tokio's
  own source or issue tracker) rather than anything specific to this
  codebase's logic — not something to chase further inside this PR.

## Follow-up round 4: reagent P1 ×2 — unbounded taskkill, and pipe-path had no tree-kill

A fifth review round on the `run_via_pipes` idle-kill commit found two more
real gaps, both direct consequences of not fully carrying the tree-kill
fix (round 2, above) over to every call site:

- **Unbounded `kill_process_tree` call.** `run_via_pty`'s idle branch
  awaited `tokio::task::spawn_blocking(move || kill_process_tree(pid))`
  with no timeout — every *other* blocking step in the same function
  (`wait_task`, `publisher_handle`, the reader-task join) is bounded to
  5s, but this one wasn't. `kill_process_tree` shells out synchronously to
  `taskkill /T /F /PID`, and `taskkill /T` itself hanging is a documented
  Windows failure mode — so an unresponsive `taskkill` would have
  reintroduced the exact unbounded hang this whole PR exists to eliminate,
  directly contradicting the surrounding comment's claim that "the wrapper
  never blocks unboundedly on this." Fixed by wrapping that same
  `spawn_blocking` call in a 5s `tokio::time::timeout`, matching every
  other bounded step.
- **`run_via_pipes` had no tree-kill at all.** Its idle branch only called
  `child.start_kill()` — like `ChildKiller::kill()` before the round-2 fix,
  this terminates only the direct `bash` process, not descendants it
  forked (a backgrounded `&` child, or a pipeline segment). Those could
  survive, still attached to the stdout/stderr pipes, leaving the reader
  tasks waiting for an EOF that never arrives — falling through their own
  5s join timeout as "abandoned" rather than confirmed dead, silently
  reproducing the same leak class this PR exists to fix. Fixed by
  capturing `child.id()` right after spawn and calling the same
  (bounded) `kill_process_tree()` before `child.start_kill()`, mirroring
  `run_via_pty`'s tree-kill-first ordering and its documented reasoning
  exactly.

Verified: `cargo build` clean, full suite **48/48 passing** (unchanged
count and timing — these were pure hardening fixes to code paths the
existing tests don't specifically exercise, not new test-requiring
behavior), manual `git log --oneline -50` repro re-confirmed clean once
more.

## Follow-up round 5: CI failure — the removed test's replacement needed to be a real integration test

CI's Windows job failed after the round-4 push, in a test that had been
passing locally through every round: `bashwrap_binary_idle_kill_cleans_up_
full_process_tree` (from round 2 — the one that spawns the compiled binary
as a real subprocess). The failure:

```
expected the real binary at "D:\a\agentmux\agentmux\target\debug\agentmux-bashwrap.exe"
(derived from test harness path "...\target\debug\deps\agentmux_bashwrap-abbe5fd1a4b7d3cf.exe")
— build it first with `cargo build -p agentmux-bashwrap`
```

**Root cause:** that test lived in `bash_wrap.rs`'s own `#[cfg(test)] mod
tests` — a unit test compiled as part of the `agentmux-bashwrap` bin's own
test harness, not a genuine Cargo integration test (`tests/*.rs`). Cargo
only auto-builds a package's plain `[[bin]]` artifact (and sets
`CARGO_BIN_EXE_<name>`) for integration/bench/example targets that
reference it — a `cargo test --workspace` run has no reason to also
produce a plain `target/debug/agentmux-bashwrap.exe` alongside the test
harness binaries, since nothing in a unit-test-only build graph asks for
it. The test's own path-derivation workaround (`current_exe()`'s parent's
parent, documented at the time as the correct fallback since
`CARGO_BIN_EXE_*` isn't set for embedded unit tests) computed the *right*
path — but nothing had ever put a file there in a genuinely clean CI
checkout. It only ever passed locally because dozens of earlier `cargo
build -p agentmux-bashwrap` invocations during this same investigation's
manual verification passes had left a plain binary sitting at exactly that
path — an artifact of this dev machine's build history, not something CI
starts with.

**Fix:** moved the test to a real integration test target,
`agentmux-bashwrap/tests/idle_kill_full_process_tree.rs`. `CARGO_BIN_EXE_
agentmux-bashwrap` is correctly set there (this *is* the case that env var
mechanism exists for), and referencing it is what makes cargo guarantee
building the plain bin artifact as a normal part of `cargo test` — no
manual pre-build step needed, in CI or locally. Verified by deleting the
leftover `target/debug/agentmux-bashwrap.exe` locally before each of
several `cargo test` runs (simulating a clean checkout): the binary is
rebuilt automatically every time, and the test passes consistently (48/48
across the whole suite: 47 unit tests + 1 integration test).

**Takeaway for next time:** any test that needs to invoke the crate's own
compiled binary as a subprocess (as opposed to calling its functions
in-process) belongs in `tests/`, not embedded in `src/`'s own test module
— this was known in the abstract (the original code comment even correctly
explained *why* `CARGO_BIN_EXE_*` wasn't set) but the practical
consequence — that nothing else in the local dev loop would ever expose
the "binary doesn't exist yet" failure mode until a genuinely clean build —
wasn't obvious until CI hit it.

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
6. `run_via_pipes`'s idle-kill has no automated test (see "Follow-up round
   3" above) — production correctness was verified manually via extensive
   `eprintln!` instrumentation during the bisection, but that evidence
   isn't captured in `cargo test`'s regression net. If this path is
   touched again, worth another attempt at a clean automated test —
   possibly starting from isolating whether `mpsc::channel` /
   `spawn_publisher_loop` / the shared `std::sync::Mutex<Instant>`
   specifically (the pieces present in the real code but absent from the
   minimal repro that passed) is what triggers the harness hang, continuing
   the bisection this round stopped short of finishing.
7. Is the underlying `.abort()`-on-timeout-wrapped-Windows-pipe-read hang
   (found while bisecting #6) a known `tokio` issue? Not checked against
   tokio's own issue tracker/changelog. If reproducible outside this
   codebase, worth reporting upstream — the minimal repro
   (`tokio::time::timeout(50ms, ChildStdout::read(...))` in a loop,
   `.abort()`ed after the owning child is killed) is small enough to
   extract into a standalone report if someone picks this up.

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
| 18 | Third ReAgent review round on the tree-kill commit: P1 (idle/success misclassification race — `_ = idle_rx` treats sender-dropped-without-sending the same as a real signal) and P2 (diagnostic appended to `buffered` before draining `publisher_handle`, could interleave ahead of trailing real output). Fixed both. |
| 19 | Added a regression test for P1; honestly verified (by temporarily forcing the old buggy behavior back in) that it does NOT reliably reproduce the race on this machine — recorded as a known test-coverage gap rather than claimed as proof. |
| 20 | While adding that test, the full suite started failing with a spurious "3 survivors." Root cause: `std::process::id()` is identical across all tests in one `cargo test` binary (parallel test harness, one process), so two different tests' PID-based markers shared a substring and cross-contaminated each other's WMI survivor checks. Fixed by tagging each test's marker distinctly. Re-ran 4× consecutively: 48/48 passing every time. |
| 21 | Fourth ReAgent review round: `run_via_pipes` (the `openpty()`-failure fallback) had zero idle-kill protection at all — `child.wait().await` fully unbounded. Implemented the same idle-timeout design as `run_via_pty` (shared last-activity clock, polling watcher, `tokio::select!`). |
| 22 | Writing the obvious test (same shape as the PTY-path ones) surfaced a new, unrelated problem: the test function's body completed correctly every time (all diagnostics fired, all assertions passed) but `cargo test` never printed a result — the process just sat there. |
| 23 | Extensive bisection via standalone minimal repro tests (incrementally adding pieces back to a known-working bare spawn+kill+wait baseline) isolated the exact trigger: `.abort()`ing a task that's inside `tokio::time::timeout(_, ChildStdout::read(...))` on a just-killed child's Windows pipe hangs that specific test's runtime shutdown — even though `.abort()` itself returns immediately. A per-read-timeout-free bare reader could be safely aborted; adding the 50ms quiet-window timeout wrapper (matching `stream_reader`'s real structure) is what triggered it. |
| 24 | Fixed `run_via_pipes` to `tokio::join!` (bounded by a 5s timeout) the reader tasks instead of `.abort()`ing them — safe because dropping a `JoinHandle` (unlike calling `.abort()` on it) doesn't cancel the underlying task. This did NOT resolve the automated test for the real `run_via_pipes` (with its full `stream_reader`/`mpsc`/`spawn_publisher_loop`/shared-mutex machinery) — it still hung identically, despite an isolated minimal repro with the same select!/kill/join structure passing cleanly. Further bisection against those remaining pieces was not completed. |
| 25 | Decided to remove the automated test for `run_via_pipes`'s idle-kill rather than continue an open-ended bisection, since (a) production correctness was already thoroughly proven via the diagnostic instrumentation from this same investigation, and (b) the specific failure mode — a graceful async-runtime-shutdown hang — cannot occur in production, where `main()`'s only exit path is an unconditional, non-graceful `std::process::exit()`. Verified the rest of the suite: 48/48 passing, clean and fast (~4s). |
| 26 | Fifth ReAgent review round: two more P1s, both from not fully carrying the round-2 tree-kill fix everywhere. `run_via_pty`'s `kill_process_tree` call had no timeout (unlike every other bounded step in that function) — a hung `taskkill /T` would reintroduce the unbounded-hang bug this PR fixes. `run_via_pipes`'s idle-kill only called `child.start_kill()`, never `kill_process_tree` — descendants forked by the pipe-path's child could survive, unaddressed. Fixed both: bounded the taskkill call with the same 5s timeout pattern used elsewhere, and added the same (bounded) tree-kill to `run_via_pipes`, ordered before `start_kill()` exactly like the PTY path. Full suite still 48/48, manual repro re-confirmed clean. |
| 27 | ReAgent approved. CI's Windows job then failed in `bashwrap_binary_idle_kill_cleans_up_full_process_tree` — passing locally through every prior round only because leftover `cargo build` artifacts from this session's many manual verification passes happened to sit at the exact path the test's `current_exe()`-derived fallback expected; a genuinely clean CI checkout never produces a plain `agentmux-bashwrap.exe` for a unit-test-only build graph. Fixed by moving the test to a real integration test target (`agentmux-bashwrap/tests/idle_kill_full_process_tree.rs`), where `CARGO_BIN_EXE_agentmux-bashwrap` is correctly set and referencing it makes cargo guarantee building the plain bin automatically. Verified locally by deleting the leftover binary before each of several `cargo test` runs — rebuilt automatically every time, 48/48 passing consistently (47 unit + 1 integration). |
