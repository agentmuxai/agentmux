# Retro: `task dev` killed 4× — Bash-tool SIGINT + mid-build worktree mutation

**Date:** 2026-09-05
**Status:** Implemented — root-caused, verified live (`task dev` launched
successfully via `mcp__agentmux__Shell`), and the CLAUDE.md guidance hardened
in the same PR. No code change was needed: the build system was never broken.
**Severity:** Low (build system was never broken) / Medium as a time sink — ~50 minutes of agent time burned, and the agent wrongly told the user the build system was at fault
**Observed by:** Agent2 (Claude agent) during the cross-pane-input-delay session (PRs #2973, #2976)
**Related retros:**
`retro-task-dev-agent-shell-path-2026-06-27.md` (Gap A/B — MSYS2 `.cmd` resolution, cmd.exe PATH),
`retro-task-package-mcp-timeout-and-shell-output-gap-2026-08-06.md` (Bash-tool timeout, `nohup` not detaching)

---

## TL;DR

An agent tried to launch `task dev` four times so the user could test a UI change.
All four died. The agent concluded the build was being killed by "resource
contention from other agents on a shared machine," told the user so, and gave up.

**That diagnosis was wrong, and the build system was never at fault.** Running
`cargo build --release -p agentmux-srv` directly succeeded on the first try
(exit 0, 4m39s). The real causes were two things the agent itself did:

1. **Launching via the Bash tool with `&`** instead of `mcp__agentmux__Shell`.
   When the Bash tool call returns, the harness terminates that call's process
   group — the detached `task dev` gets **SIGINT**. Signature: a literal `^C` in
   the redirected log, then `error: could not compile` and go-task
   `exit status 58`.
2. **Merging a PR while a build was running against the same worktree.**
   `gh pr merge --delete-branch` checks out `main` and fast-forwards it,
   swapping files out from under an in-flight `cargo build`.

Relaunching through `mcp__agentmux__Shell` — **exactly what CLAUDE.md already
documents** — worked first try: full build, Vite up, host PID 64584, authkey
written.

The most important finding is not either bug. It is that **CLAUDE.md already
contained the correct instructions, and the agent didn't follow them.**

---

## Timeline

| # | Launch method | Outcome |
|---|---|---|
| 1 | Bash tool, `&`, `run_in_background: true` | Instance came up. Used successfully for the cross-pane benchmark. |
| 2 | Bash tool, `&`, `run_in_background: true` (baseline build, fix stashed) | Never came up. Abandoned after ~30 min; blamed on "build contention." |
| 3 | Bash tool, `&`, `run_in_background: true` (`TITLE=Agent2`) | Never came up. Log stayed 0 bytes. |
| 4 | Bash tool, `&` + `sleep 3`, **no** `run_in_background` | Died at `Compiling agentmux-srv`. Log ends `^C` + `exit status 58`. |
| 5 | Bash tool, `&` + `sleep 3`, no background | Same failure, same place. |
| 6 | Bash tool, `&` + `sleep 3`, no background | Same failure, same place. |
| — | **Direct `cargo build --release -p agentmux-srv`** | **Exit 0. 4m39s. Clean.** |
| 7 | **`mcp__agentmux__Shell` + `scripts\dev-agent.cmd`** | **Worked. Vite ready, host PID 64584.** |

---

## Root cause 1 — Bash-tool `&` does not detach; the process group is SIGINT'd

The Bash tool's shell is harness-managed. A background job started with `&`
inside a tool call belongs to that call's process group, and the group is
terminated when the call returns. `task dev` is a multi-minute build; every
tool call returns in seconds. The build is always killed mid-flight.

**Diagnostic signature** (all three of these together):

```
task: [build:backend:rust:windows] cargo build --release -p agentmux-srv
   Compiling agentmux-srv v0.55.34 (...)
^Cerror: could not compile `agentmux-srv` (bin "agentmux-srv")
task: Failed to run task "dev": ... exit status 58
```

- A literal **`^C`** immediately before the error — this is the tell. A real
  compile error is preceded by a `error[EXXXX]:` diagnostic with a file/line.
  A bare `error:` with no diagnostic above it means the compiler was *signalled*,
  not that it *rejected* anything.
- **`exit status 58`** from go-task.
- Failure always at the same place (whatever was compiling when the call
  returned) — which superficially *looks* like a deterministic compile error and
  is what misled the diagnosis for three attempts.

**Why attempts 1–3 look different (0-byte logs, no `^C`):** with
`run_in_background: true` the wrapper returns in ~0.1s, so the child is orphaned
rather than signalled — sometimes it survives (attempt 1 did), sometimes it dies
before flushing anything to the redirect (attempts 2–3, 0-byte logs). **This
non-determinism is the trap**: attempt 1 succeeding is what made the agent
believe the method was sound and look for an external cause when it later failed.

### The fix (already documented, not followed)

`CLAUDE.md` → "Launching `task dev` from an agent / MCP Shell (Windows)":

```json
{ "cmd": "C:\\<repo>\\scripts\\dev-agent.cmd TITLE=my-branch > C:\\<repo>\\devrun.log 2>&1" }
```

`mcp__agentmux__Shell` runs server-side via `ShellNodeRunner` — a genuinely
detached process with an independent lifetime, unaffected by tool-call
boundaries. The `Shell` tool's own description states its purpose plainly:
*"Use for build systems, watchers, dev servers — anything that should run in the
background without blocking the conversation."*

Verified: launched via MCP Shell, survived multiple tool-call boundaries, zero
`^C` in an 84KB log, completed the full build, Vite ready, instance live.

---

## Root cause 2 — `gh pr merge` mutates the worktree an in-flight build is reading

Attempt 4's log opened with a *different* failure than 5 and 6:

```
task: [npm:install] npm install
^Cerror: could not compile `agentmux-srv`
task: Failed to run task "npm:install": exit status 58
```

`gh pr merge --squash --delete-branch` (run while that build was live) does a
local `checkout main` + fast-forward + branch delete. Rewriting tracked files
under a running `cargo build` / `npm install` corrupts the build. It also left a
stray 133-line `package-lock.json` diff in the worktree that had to be reverted.

**Rule:** never run `git merge` / `git checkout` / `gh pr merge` while a build is
running against the same worktree. Either stop the build first, or use a
separate worktree (`git worktree add`) — the repo already recommends worktrees
for concurrent work in the commit-hygiene guidance.

This is distinct from root cause 1 and would have broken the build even from a
correctly-detached MCP Shell.

---

## Why the wrong conclusion was reached

Worth recording, because the technical bugs are the cheap part:

1. **A plausible-but-unverified theory was adopted early.** "Shared machine,
   other agents building, ~28GB free but something's killing it" fit the
   evidence loosely and was never tested. It became the working assumption for
   three attempts.
2. **The `^C` was visible in the log and read past — twice.** It appears in the
   first failing log and again in a `tail -c 2000`. It is the single most
   diagnostic character in the whole output.
3. **The obvious control experiment was run last, not first.** Running
   `cargo build` directly takes 5 minutes and immediately separates "build is
   broken" from "something is killing the build." It was the step that solved
   it, and it should have been step one — this is exactly what the
   systematic-debugging skill's *"reproduce it / read the actual error"* steps
   prescribe.
4. **The project's own docs were not consulted.** CLAUDE.md has a section with
   this literal title: *"Launching `task dev` from an agent / MCP Shell
   (Windows)"*. Two prior retros cover adjacent failures in the same area. The
   agent had `mcp__agentmux__Shell` available the whole time and used the Bash
   tool instead.
5. **The user was given a wrong root cause and told to work around it.** The
   agent recommended the user run `task dev` themselves — externalizing a
   self-inflicted problem. The user's pushback (*"we've done a lot to make sure
   task dev runs robust and isolated"*) was correct and was the thing that
   forced a real diagnosis.

---

## Prevention

### Immediate (docs)

- Add the **`^C` + `exit status 58`** signature to CLAUDE.md's existing
  "Diagnosing failed shells" list — the current table covers `line_count:2`/
  `exit:1` and `line_count:53`/`exit:200` (both *instant* failures) but has no
  entry for a build that starts fine and is killed minutes in, which is what a
  Bash-tool launch produces.
- State explicitly in that section: **the Bash tool cannot launch `task dev`,
  with or without `&`, with or without `run_in_background`.** The current wording
  says to use `scripts\dev-agent.cmd` but doesn't say the Bash tool is a dead end
  — and attempt 1 proves it fails *intermittently*, which is worse than failing
  reliably.
- Add a "don't mutate the worktree during a build" note next to the `gh pr merge`
  guidance in the Git Workflow section.

### Behavioural (for agents)

- **Before concluding a build is externally broken, run the build command
  directly.** One 5-minute control run beats three 10-minute guesses.
- **A bare `error:` with no `error[EXXXX]` diagnostic above it is a signal, not a
  compile failure.** Look for `^C` / signal evidence.
- **Grep CLAUDE.md and `docs/retro/` before declaring an infrastructure problem
  novel.** Both the tool to use and two adjacent retros were already on disk.

### Possible follow-up (code)

- `scripts/dev-agent.cmd` could detect it's been signalled and print a one-line
  hint (`"interrupted — if launched from an agent, use mcp__agentmux__Shell"`)
  rather than letting go-task's generic `exit status 58` be the last word. Low
  effort, would have collapsed this whole episode into one attempt.

---

## Verification

Root cause confirmed, not merely theorised:

- `cargo build --release -p agentmux-srv` standalone → **exit 0**, 4m39s, no error.
  Proves the build is healthy and the failures were external to it.
- Same command via Bash tool + `&` → `^C`, `exit status 58`, 3/3 reproductions,
  always at the point the tool call returned.
- Same command via `mcp__agentmux__Shell` → survived multiple tool-call
  boundaries, 84KB of clean log, **0 occurrences of `^C`**, build completed,
  Vite ready on `localhost:5297`, authkey written to
  `~/.agentmux/dev/main/71037a8bbd2cda52/data/`, host PID 64584 confirmed alive
  in `tasklist`.

The isolation/robustness work in `task dev` itself needed no changes — it did
its job correctly every time it was allowed to run to completion.
