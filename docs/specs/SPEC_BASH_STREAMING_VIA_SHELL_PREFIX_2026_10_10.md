# SPEC: stream Bash output through Claude Code's shell prefix instead of rewriting the command

**Status:** proposed
**Date:** 2026-10-10
**Replaces (for Claude Code):** the command rewrite in `SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §2 and §5.
**Background:** `docs/retro/RETRO_BASHWRAP_HOOK_DROPS_BASH_TOOL_FIELDS_2026_10_10.md`.

## 1. The problem

AgentMux streams a Bash command's output into the agent pane by rewriting the
command in a `PreToolUse` hook: `cargo build` becomes
`agentmux-bashwrap exec --tool-id=… --b64-cmd=…`, and the CLI runs that.
Rewriting the command means the CLI never sees it:

- **The CLI's handling of the call** applies to the wrapper: the task label,
  the summary and the safety checks all see `agentmux-bashwrap exec --b64-cmd=…`.
  When the hook dropped the call's other fields, background, timeout and
  description were lost too (fixed in #4570).
- **Permission rules.** The hook answers `permissionDecision: "allow"` for
  every call. That takes precedence over the user's own `deny` and `ask` rules in
  their Claude settings. Removing `allow` doesn't help: the rules are then
  matched against the rewritten command, so `Bash(rm:*)` never matches.
- **It depends on how the CLI treats `updatedInput`,** which changed once
  without notice.

## 2. The design

Claude Code runs every Bash tool command as `<CLAUDE_CODE_SHELL_PREFIX> '<script>'`
when that environment variable is set. The tool call itself is untouched. AgentMux
sets the prefix to its own `agentmux-bashwrap`, which runs the script the way
`exec` runs a command today: in a PTY, streaming each line to the pane.

```
model: Bash {command: "cargo build", description, timeout, run_in_background}
  │
  ├─ PreToolUse hook: `agentmux-bashwrap hook`
  │     records (session, tool_use_id, command, run_in_background); changes nothing,
  │     decides nothing
  │
  ├─ the CLI's own permission rules and checks see "cargo build"
  │
  └─ the CLI runs: agentmux-bashwrap '<the CLI script, with eval '\''cargo build'\''>'
        prefix mode: claims the record for this command → tool_use_id
        runs the script in a PTY, streams chunks keyed by tool_use_id,
        prints "<exited N in Xs>" + output, exits with the script's code
```

### 2.1 What the CLI does with the prefix (verified on the pinned 2.1.288)

Checked against the real CLI with a fake API (`scripts/cli-contract/`):

| Question | Answer |
|---|---|
| Which commands get the prefix? | Every Bash tool command, background ones included. Not hooks, and not the CLI's own setup. |
| What does it receive? | One argument: the CLI's whole script, i.e. `source <snapshot> … && eval '<command, single-quoted>' < /dev/null && pwd -P >| <cwd file>`. |
| Does the tool call keep its fields? | Yes: the call goes to the background at once, the task is labelled with the description, and the summary uses it. |
| Do permission rules see the real command? | Yes: `deny: Bash(touch:*)` blocks `touch …`. |
| Can the prefix have arguments? | No: the whole value is quoted as one path. A path with spaces works. |
| How does the prefix find the call's `tool_use_id`? | It doesn't get it. It does get `CLAUDE_CODE_SESSION_ID`, which equals the hook payload's `session_id`. |
| Does `cd` carry over between calls? | Not by the CLI alone (it resets the directory, with or without the hook). Today bashwrap's own saved-directory file makes it carry over; prefix mode keeps that file. |

### 2.2 Linking a run to its tool call

1. **The hook records each call.** It writes
   `<state>/bashwrap-calls/<session_id>/<tool_use_id>.json` with the command,
   `run_in_background` and a timestamp. It returns `{}`: no `updatedInput`, no
   `permissionDecision`.
2. **The prefix claims a record.** It reads `CLAUDE_CODE_SESSION_ID` and takes
   the command out of the script's `eval '…'` (undoing the CLI's `'"'"'`
   quoting). Then it claims the oldest unclaimed record for that session with
   exactly that command, by renaming the file, which is atomic. Two identical
   commands running at once may swap records. Both are the same command, so the
   output is still right, at worst on the other card.
3. **When there's no match** (the eval can't be parsed after a CLI change, or
   the record is missing), it uses the session's only unclaimed record if there
   is exactly one. Otherwise it runs the script with no tool id: no live output,
   but the command still runs and returns its result. It never fails a command
   just because it can't be linked.
4. Records older than an hour are deleted on each claim.

### 2.3 What prefix mode keeps from `exec`

Everything `exec` does after it has the command, run on the CLI's script
instead:
- the PTY, with the pipe fallback;
- Git Bash discovery;
- the idle kill and process-tree kill, and the background exemption from the
  record's `run_in_background`;
- `setsid` and the pid report for background tasks;
- the saved working directory;
- live chunks to `block:<AGENTMUX_BLOCKID>` keyed by `tool_use_id`, with the
  terminal marker;
- the `<exited N in Xs>` header and the 50 + 50 KB cap;
- the access-denied hint;
- exiting with the script's code.

The **tee rewrite** (a trailing `> file` becomes `| tee file`) is applied to the
command taken out of the `eval`, and the script is rebuilt with it. If the
command can't be taken out, the script runs as is (no tee).

### 2.4 How it's switched on

- **srv** sets `CLAUDE_CODE_SHELL_PREFIX` to the bundled `agentmux-bashwrap`
  (an absolute path, never a PATH lookup: see
  `RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md`) in the Claude provider's spawn
  environment.
- **The hook** registers instead of rewriting when its own environment has
  `CLAUDE_CODE_SHELL_PREFIX` pointing at an `agentmux-bashwrap`. Without it
  (an older srv, or someone running `claude` by hand in an agent's workspace),
  it rewrites exactly as today. So a new hook never strands a command.
- **The binary** recognises a prefix call before parsing its subcommands, as it
  already does for askpass: exactly one argument, which isn't a subcommand or a
  flag.
- **Kill switch:** `AGENTMUX_BASHWRAP_MODE=rewrite` in srv's environment keeps the
  old behaviour.
- Container agents get the in-image path (`/usr/local/bin/agentmux-bashwrap`).
  Streaming from containers is blocked today by the container route allowlist;
  that is out of scope here.

### 2.5 What changes for users

- Their own `deny` and `ask` rules for Bash apply inside AgentMux. AgentMux's
  own permission channel still allows ordinary tools, so nothing new is asked.
- The CLI's labels, summaries and checks see the real command.
- Everything else looks the same: live output, the exit pill, `cd` carrying
  over, background tasks and their stop.

## 3. Plan

1. **bashwrap prefix mode** (`crates/bashwrap`):
   - `main.rs`: detect the prefix call;
   - `prefix.rs`: take the command out of the script, records, claims;
   - `bash_wrap.rs`: run a script instead of a decoded command;
   - `hook.rs`: register when the prefix is on.
   - Unit tests for the parsing, the claims and the hook's two modes.
2. **srv**: set the prefix in the Claude spawn environment (persistent and
   one-shot), with the kill switch. The Rust and TS settings builders stay as
   they are; their existing `allow` rule for `agentmux-bashwrap` only matters to
   the old path.
3. **Contract test**: prefix-mode cases against the real CLI:
   - background with its description;
   - timeout;
   - output through the wrapper and linked to the call (the record is claimed);
   - a `deny` rule honoured;
   - `cd` carrying over across two calls.

   Keep the rewrite-mode cases for the fallback.
4. **Check in a dev build**: a real agent pane streams output, the exit pill
   shows, a background `task dev` can be stopped, and Bash doesn't prompt.
5. **Docs**: amend `SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` to point here.

## 4. Risks

- **The CLI's script format can change.** Taking the command out of the `eval`
  depends on it. A change degrades to the fallback in §2.2 (no tee, possibly no
  live output), never a failed command. The contract test catches it on the pin
  bump.
- **The prefix variable could be removed or changed.** The contract test checks
  it; the hook falls back to rewriting if the prefix isn't in its environment.
- **Windows argument handling.** Git Bash may convert path-like arguments to a
  native exe. The script starts with `source`, so it isn't converted, but this
  is checked in the dev build.
- **Parallel identical commands** may show on each other's cards (§2.2).
