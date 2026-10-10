# Retro: the Bash hook drops every field but `command`, so the CLI shows wrapped commands and ignores background and timeout (2026-10-10)

**Status:** retro
**Reported by:** the operator, from an agent pane's transcript.
**Affects:** every Claude Code agent whose Bash calls go through
`agentmux-bashwrap hook` (all of them, by default).
**Fixed:** in the PR that adds this retro: §6 items 1, 2 and 4. Item 3 is
open; it was checked by hand for this PR (§6).

## 1. What the operator saw

After a background build finished, the agent pane showed this line:

```
Woke up: Background command "agentmux-bashwrap exec --tool-id=toolu_01228M… --b64-cmd=Y2QgIiRBR0VOVE1V…bG9n --declared-background" completed (exit code 0)  ambient
```

The trailing `ambient` is not part of the defect: it is the pane's tag for a
line AgentMux wrote rather than the model (`AmbientNarrationBlock.tsx`,
`AMBIENT_TAG_LABEL`), kept on purpose when the line is copied. The defect is
the text before it. The agent had called Bash with the description "Build fresh
portable build onto the Desktop". The line should read
`Woke up: Background command "Build fresh portable build onto the Desktop" completed (exit code 0)`.
Instead it shows AgentMux's internal wrapper with the real command
base64-encoded inside it.

## 2. Root cause

AgentMux streams Bash output by rewriting each Bash call in a `PreToolUse`
hook (`SPEC_STREAMING_BASH_RUNNER_2026_05_11.md`). `build_response` in
`crates/bashwrap/src/hook.rs` returns:

```json
{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow",
  "updatedInput": {"command": "agentmux-bashwrap exec --tool-id=… --b64-cmd=… [--declared-background]"}}}
```

Claude Code **replaces** the tool's arguments with `updatedInput`; it does not
merge. The hooks reference says so directly: "`updatedInput` directly under
`hookSpecificOutput` replaces a tool's arguments before it runs." So the call
that actually runs has only `command`. The Bash tool's other fields are gone:

| Field the model set | What the CLI does without it |
|---|---|
| `description` | Labels the call, and its background task's `task_started` / `task_notification`, with the **command**: here, the wrapper. |
| `run_in_background` | Runs the command in the **foreground**. |
| `timeout` | Uses the default 120 s; when that runs out it moves the command to the background. |
| `dangerouslyDisableSandbox` | Dropped too. No effect seen on Windows, but the same mechanism applies. |

The hook does read `run_in_background` and passes it to the wrapper as
`--declared-background` (#2589). That exempts a declared long-runner from
bashwrap's idle kill, but the CLI itself never sees the flag.

## 3. Evidence

From one agent's own transcript, for the build call above (CLI 2.1.288):

- The model's `tool_use` input:
  `{"command": "cd … && bash scripts/package.sh …", "description": "Build fresh portable build onto the Desktop", "timeout": 7200000, "run_in_background": true}`.
- The hook's recorded stdout (`hook_success`, `PreToolUse:Bash`): `updatedInput`
  with `command` only, as in §2.
- The tool result: "Command did not complete within its 120s timeout and was
  moved to the background", with `toolUseResult.timedOutAfterMs: 120000`. So
  both `timeout` and `run_in_background` were ignored.
- The `task_notification` summary: `Background command "agentmux-bashwrap exec …" completed (exit code 0)`.
  This is the string the pane's wake line repeats.

Across every agent transcript on the same machine (all provider accounts),
counting Bash calls that set `run_in_background: true` and that the hook
rewrote, by CLI version:

| CLI version | First seen | Went to the background at once | Ran in the foreground |
|---|---|---|---|
| 2.1.198 | 2026-07-30 | 142 | 33 |
| 2.1.247 | 2026-08-30 | 0 | 71 |
| 2.1.263 | 2026-09-06 | 0 | 343 |
| 2.1.274 | 2026-09-18 | 0 | 79 |
| 2.1.280 | 2026-09-29 | 0 | 224 |
| 2.1.285 | 2026-09-29 | 0 | 286 |
| 2.1.287 | 2026-09-30 | 0 | 433 |
| 2.1.288 | 2026-10-04 | 0 | 530 |

The two calls in the same data that the hook did *not* rewrite (sessions
without it) both went to the background at once. For one account, the
foreground runs split into 109 that finished in the foreground and 249 that
hit the 120 s default and were moved to the background.

It reproduced live while this retro was being written: a scan given
`timeout: 300000` was cut at 120 s and moved to the background.

The CLI builds the summary from the description when it has one. srv's own
fixture for the background-task feed (`crates/srv/src/backend/background_task_feed.rs`,
test near line 450) records
`"summary": "Background command \"Background wait for agents\" completed (exit code 0)"`,
the description of that call.

## 4. Impact

1. **Transcript.** Every "Woke up:" line (#4503, `task-wake.ts`) for a Bash
   background task shows the wrapper and a base64 blob instead of what the
   agent said it was doing. History replay shows the same, because the summary
   is stored that way in the transcript.
2. **Agents block when they asked not to.** A call meant to run in the
   background holds the agent's turn for up to 120 s (`task dev`, servers,
   long builds, watchers). This happened about 2,000 times on one machine
   since late August.
3. **Long foreground commands are cut at 120 s.** A command the agent meant to
   wait for (a build with `timeout: 600000`) is moved to the background at
   120 s, and the agent has to come back for it.
4. **Background commands are stopped at 30 min.** The CLI caps a background
   task at 30 min unless the call asked for longer, and the request never
   arrives. This is consistent with two `task dev` launches earlier in the same
   session, asked for 2 h, being stopped at the background time limit.
5. **Everything keyed on the description gets the wrapper.** The background
   dock and Swarm's task labels come from the CLI's `task_started` description
   (`background_task_feed.rs`, `label = description`), so they get the command
   instead. The "started by" labels proposed in
   `SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md` §5.3 would too.

## 5. Why nobody caught it

- **Tests check our half only.** `hook.rs`'s tests assert the wrapped
  `command` and the `--declared-background` flag. Nothing asserts that the
  other `tool_input` fields survive, because the hook was written assuming the
  CLI merges `updatedInput`.
- **It fails quietly.** Commands still run, output still streams, and a
  120 s timeout just looks like the CLI backgrounding a slow command. An agent
  reads "moved to the background" as normal.
- **The pane hid the description.** The tool card shows the wrapped command
  decoded (`ToolBlock`), so the missing description wasn't visible until
  #4503 started printing the CLI's own summary.
- **The CLI changed underneath it.** The hook's response shape hasn't
  changed since #804 (2026-05-11). On CLI 2.1.198 (July 2026) most rewritten
  background calls still went to the background, so the CLI then kept at least
  `run_in_background` (merged, or read it from the original input). From
  2.1.247 (first seen 2026-08-30) none did: about 2,000 calls since then. The
  break came with a CLI upgrade between those versions, not with an AgentMux
  change, and nothing in AgentMux tests the CLI's side.

## 6. Fix

1. **The hook** (`crates/bashwrap/src/hook.rs`): start `updatedInput` from a
   copy of `tool_input` and replace only `command`, so `description`,
   `timeout`, `run_in_background` and any field added later pass through
   unchanged. Add a test that feeds a payload with all four fields and asserts
   each survives (and that a non-object `tool_input` still produces the
   command-only rewrite).
2. **The transcript, for history already written** (`task-wake.ts`): when a
   summary contains `agentmux-bashwrap exec … --b64-cmd=<b64>`, show the
   decoded command (shortened) instead. Better still, prefer the description
   from the matching `task_started` line when there is one. Old transcripts
   keep the wrapped summary forever, so the pane needs this even after the hook
   fix.
3. **Guard against a regression**: an integration test that runs the real
   CLI with the hook installed, calls Bash with `run_in_background: true` and a
   description, and asserts the result is the immediate "running in background"
   reply and the label is the description. This is the only check that catches
   the CLI changing its `updatedInput` semantics again.
4. **Docs:** `SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §5 should say that
   `updatedInput` replaces the whole argument object and the hook must pass
   the other fields through.

**Checked by hand against the real CLI** (2.1.288, `claude -p` with only the
hook configured, one Bash call with a description, `timeout: 300000` and
`run_in_background: true`):

| | Tool result | Task label (`task_started`) | `task_notification` summary |
|---|---|---|---|
| Hook before the fix | the command's output (it ran in the foreground) | `agentmux-bashwrap exec --tool-id=… --b64-cmd=… --declared-background` | the same wrapper |
| Hook after the fix | "Command running in background with ID: …" | "Wait two seconds for the hook test" | `Background command "Wait two seconds for the hook test" completed (exit code 0)` |

## 7. Lessons

- A hook that rewrites a tool call owns the **whole** call. Pass through
  every field you don't mean to change; don't rebuild the object from the one
  field you care about.
- When AgentMux sits between the model and the CLI, test what the CLI did
  (the tool result), not only what AgentMux sent.
