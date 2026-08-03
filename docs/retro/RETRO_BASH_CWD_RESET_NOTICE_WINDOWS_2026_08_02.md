# "Shell cwd was reset to ..." appearing on Windows agent panes

**Date:** 2026-08-02
**Reported by:** Asaf (via Agent3), observed on a live Windows AgentMux session
**Ground truth basis:** `agentmuxai/agentmux` `main` at commit `740303442` (pulled fresh for this
investigation), reproduced live during the investigation itself.
**Status:** Fixed — see §6.

## 0. Symptom

A line reading

```
Shell cwd was reset to C:\Users\asafe\.agentmux\agents\<agent-slug>
```

appears at the end of Bash tool-call previews in the agent pane, on Windows. Per the report, this
used to be a macOS-only occurrence; it is now also showing up on Windows.

## 1. Live reproduction (this session)

The message fired after **almost every single Bash tool call** issued in this session — not
occasionally, not only after an explicit `cd`. Concretely:

1. `cd "C:/Users/asafe/agentmux" && git status --short --branch && git remote -v` — succeeded,
   printed real output, **then** the reset line was appended.
2. `cd "C:/Users/asafe/agentmux" && git fetch origin main && git log ...` — same again.
3. A follow-up call, **without** an explicit `cd`, `git log --oneline -3` — failed with
   `fatal: not a git repository (or any of the parent directories): .git`.

(3) proves the `cd` from calls (1)/(2) did not actually persist at the OS level between tool calls
— the process was back in a directory with no `.git` above it (the agent's home directory,
`C:\Users\asafe\.agentmux\agents\agent3-0630k`, exactly the path the notice reports). This is a
real, reproducible loss of shell state, not just a cosmetic message.

## 2. The message does not come from AgentMux's own code

Exhaustive search of the full `agentmuxai/agentmux` tree (no file-type filter, `.git`/`target`/
`node_modules` excluded) for `"was reset to"`, `"was reset"`, and `"Shell cwd"` turns up **zero**
matches that construct this string. It is not assembled from split literals either — a repo-wide
grep for just `"reset to"` and `"was reset"` independently returns nothing relevant.

Conclusion: **this text is emitted by the Claude Code CLI binary itself**, as a Bash-tool
system-reminder/diagnostic, when it detects that the shell environment it's currently running a
command in doesn't match what it expected to still be true from earlier in the session (e.g. a
previously-established working directory). AgentMux does not render or compose this string
anywhere in its frontend or backend; it is passed through verbatim from the CLI's own output.

## 3. Why AgentMux's Windows path structurally cannot preserve `cwd` across Bash calls

`agentmux-srv` always installs a `PreToolUse:Bash` hook for every agent, unconditionally, on every
platform (`agentmux-srv/src/backend/agent_config.rs:146-163`, `:359-390`, doc-commented "always
includes a PreToolUse:Bash entry pointing at `agentmux-bashwrap hook` ... regardless"; mirrored in
`frontend/app/view/agent/agent-config-builder.ts`). This exists to stream live Bash output into the
agent pane (`docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md`), not to manage cwd.

What that hook actually does (`agentmux-bashwrap/src/hook.rs::build_response`):

- Intercepts the **literal text** of every Bash tool call before Claude runs it.
- Base64-encodes the original command and rewrites it to
  `agentmux-bashwrap exec --tool-id=<id> --b64-cmd=<b64>`.
- On Windows, this rewritten line is what "the shell Claude Code's Bash tool invokes on Win32"
  (`cmd.exe /C`) actually executes (`hook.rs:487-492`, doc comment).

`agentmux-bashwrap exec` (`agentmux-bashwrap/src/bash_wrap.rs`) is confirmed, both by direct source
read and by this repo's own prior investigation
(`docs/specs/REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md` §1.1), to be a
**one-shot process, not a daemon**: "each `exec` invocation is scoped to exactly one tool call,
runs for however long that call takes, and terminates. There is no `agentmux-bashwrap` daemon."

Critically, when that one-shot process spawns the *actual* inner `bash` that runs the real,
decoded command, it seeds that inner bash's working directory like this
(`agentmux-bashwrap/src/bash_wrap.rs:723-725`, unchanged since 2026-05-17):

```rust
if let Ok(cwd) = std::env::current_dir() {
    cmd.cwd(cwd);
}
```

`std::env::current_dir()` here is **bashwrap's own process cwd** — inherited unchanged from
whatever spawned it (ultimately, the agent's configured `working_directory`, i.e. the agent's home
directory). There is no mechanism anywhere in this path that reads back a cwd left over from a
*previous* tool call. The original command's own `cd X && ...` only ever affects the disposable
inner-bash process for that one call; the moment `agentmux-bashwrap exec` exits, that state is
gone. The next Bash tool call spawns a brand-new one-shot `agentmux-bashwrap exec`, seeded from the
exact same fixed default directory every time — matching the path in the reported message exactly.

In short: **on the path AgentMux wires up for every Bash tool call, there is no cwd persistence to
lose — it was never wired up to persist in the first place.** Claude Code's own Bash tool, which
expects (and normally provides) `cd` continuity across calls within a session, notices the
mismatch after essentially every call and self-corrects by emitting the "Shell cwd was reset to
X" notice — which the agent pane then faithfully displays at the end of the tool-call preview.

## 4. Why this reads as "new on Windows, previously macOS-only" — open question

Two facts argue against this being a *recent* AgentMux regression:

- The unconditional hook injection in `agent_config.rs` dates to 2026-05-11/05-12 (`bdbc44050`,
  `a20dd1006`) and hasn't materially changed since.
- The `cmd.cwd(std::env::current_dir())` seeding in `bash_wrap.rs` dates to 2026-05-17
  (`199818cdf5`) and is also unchanged.
- `agentmux-bashwrap` is built and shipped for macOS/Linux too (`Taskfile.yml`'s
  `build:host:{darwin,linux}` targets both build it), so this isn't a Windows-exclusive binary.

Given the mechanism itself is ~3 months old and cross-platform, the most likely explanation is that
this is **not** a change in AgentMux's own code, but a change in the **Claude Code CLI itself**
(external, Anthropic-side) that started actively detecting and *reporting* the cwd mismatch more
aggressively or more often — surfacing a pre-existing structural gap that was previously silent.
That would explain the platform split too: on macOS/Linux, Claude Code's Bash tool can plausibly
hold a real persistent shell process open for a session in more cases (or hits this particular
detection path only rarely, e.g. only when that persistent shell itself has to be restarted),
whereas on Windows every single Bash call already goes through the always-fresh, one-shot
`agentmux-bashwrap exec` path described in §3 — so if Claude Code's CLI now checks for this on
every call, Windows will trip it on *every* call while macOS/Linux trips it only occasionally.

This last point is **not confirmed** — I could not find a definitive commit in this repo that
changed Windows-specific behavior recently, nor visibility into Claude Code CLI's own release
notes from within this repo. Flagging as the open item for whoever picks this up:

1. Check whether a recent Claude Code CLI version bump changed Bash-tool cwd-tracking/verification
   behavior (changelog / release notes, outside this repo).
2. Confirm (e.g. via a debug build or added tracing) whether macOS/Linux agents actually exercise
   the exact same `agent_config.rs` → `hook.rs` → `bash_wrap.rs` path per Bash call, or whether
   Claude Code's CLI behaves differently per-OS in a way that happens to paper over the missing
   persistence on non-Windows today.

## 5. Recommendation

The durable fix is to make `cwd` actually persist across `agentmux-bashwrap exec` invocations
within one agent session — e.g. have `hook.rs` (or `bash_wrap.rs`) read/write a small per-session
state file (last known cwd) next to wherever the session's other per-session state already lives,
so each new one-shot invocation seeds `cmd.cwd()` from the *previous* call's ending directory
instead of always resetting to the agent's static home directory. This is a real fix, not a
suppression of the CLI's notice — the notice is arguably correct today (the shell genuinely was
reset, every time).

## 6. Resolution

Implemented in `agentmux-bashwrap/src/bash_wrap.rs` (both `run_via_pty` and `run_via_pipes`):

- A new `CwdState::load()` resolves a per-agent state file at
  `~/.agentmux/state/bashwrap-cwd/<AGENTMUX_AGENT_ID>.cwd` (overridable via
  `AGENTMUX_BASHWRAP_CWD_STATE_FILE`, mainly for tests), and restores the previously-persisted
  directory as the inner bash's starting cwd if it still exists — falling back to today's old
  behavior (`std::env::current_dir()`) otherwise.
- `append_cwd_capture` wraps the executed command so that, after it finishes, the shell's own
  `$PWD` (captured via `pwd -W` on Windows to get a Windows-native path Rust's `Path::is_dir()` can
  actually resolve, vs. Git Bash's default MSYS-style `/c/...` output) is written back to that same
  state file — atomically (temp file + rename) — before re-exiting with the original command's real
  exit code, so this bookkeeping never changes what Claude sees as the command's result.
- Because a bash brace group (`{ ... ; }`, not `( ... )`) does not run in a subshell, a `cd` inside
  the wrapped command really does change the wrapping `bash -c` process's own `$PWD`, so this
  capture is correct even though the user's command itself is otherwise opaque to bashwrap.

This does not change what Claude Code's own CLI internally believes about the session (that's
out-of-process); it makes the *actual* directory a `cd` leaves the agent in survive to the next
Bash tool call, which is the part that was silently broken. Verified end-to-end, both via a new
`run_via_pipes_persists_cwd_across_invocations` test and by manually invoking the built
`agentmux-bashwrap.exe` twice in a row and confirming the second, independent process reports the
directory the first one `cd`'d into.

## Key files

| File | Role |
|---|---|
| `agentmux-srv/src/backend/agent_config.rs:146-163,359-390` | Unconditionally injects the `PreToolUse:Bash` → `agentmux-bashwrap hook` entry into every agent's `.claude/settings.json` |
| `agentmux-bashwrap/src/hook.rs` | Rewrites every Bash tool call into `agentmux-bashwrap exec --b64-cmd=<original>`; doc comment confirms `cmd.exe /C` is the Win32 invocation shell |
| `agentmux-bashwrap/src/bash_wrap.rs:723-725` | Seeds the one-shot inner bash's cwd from bashwrap's own inherited process cwd — the exact point where prior-call `cd` state is lost |
| `agentmux-bashwrap/src/main.rs` | Confirms `hook`/`exec` are the only two subcommands; no daemon, no persistent state between invocations |
| `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` | Original motivation for the always-on hook (live tool-output streaming), unrelated to cwd |
| `docs/specs/REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md` | Independent prior confirmation that `agentmux-bashwrap exec` is one-shot, not a daemon |
