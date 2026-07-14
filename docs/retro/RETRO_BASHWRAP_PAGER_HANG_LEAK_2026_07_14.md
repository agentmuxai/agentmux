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

## Mitigation options (not yet applied)

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

- [ ] Decide on and implement a fix (Option A and/or B) — deferred; this
  retro was written first per explicit direction this session.
- [ ] Once fixed, manually verify: run a bashwrap-wrapped `git diff` /
  `git log` on a diff large enough to normally trigger paging, and confirm
  the wrapper process exits promptly instead of leaking.
- [ ] Consider a cleanup pass to kill the currently-leaked processes found
  in this investigation (12 identified at time of writing) — not done as
  part of this retro, since killing running processes wasn't in scope for a
  documentation-only pass.
- [ ] If Option C is pursued, connect it to the existing WPS "starting"
  system-chunk mechanism (`agentmux-srv/src/server/mod.rs`) rather than
  building a second, separate tracking mechanism.

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
