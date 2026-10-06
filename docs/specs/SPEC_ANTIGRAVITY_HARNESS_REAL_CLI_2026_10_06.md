# Spec: Make the Antigravity harness drive the real `agy` CLI

**Status:** proposed
**Date:** 2026-10-06
**Author:** Camper

## Ask

Antigravity agents should launch and work. #4403 made AgentMux find an
installed `agy` instead of failing on an npm package that never existed. This
spec covers the next layer: everything AgentMux passes to `agy` and reads back
from it was copied from Gemini CLI and never checked against the real CLI.

## What `agy` actually does

Checked against `agy` 1.1.11 (Windows, installed by Google's installer) on
2026-10-06, with `agy --help`, `agy models` and two one-word prompts.

| AgentMux today (`providers.rs` `ANTIGRAVITY`, `catalog.ts`) | `agy` 1.1.11 | Verdict |
|---|---|---|
| `--output-format stream-json` | exists | OK |
| `--yolo` | no such flag; auto-approve is `--dangerously-skip-permissions` | wrong |
| `-p ""` with the prompt on stdin | `-p`/`--print` takes the prompt as an argument; stdin is ignored, and an empty `-p` fails with `Error: empty prompt` | wrong |
| resume `-r <id>` | no such flag; resume is `--conversation <id>` (`-c` continues the most recent) | wrong |
| session id field `session_id` | the id is `conversation_id` | wrong |
| translator `gemini-json` | a different event schema (below) | wrong |
| auth check `auth status`, login `auth login` | no `auth` subcommand; sign-in happens on first run (system keyring, falling back to Google Sign-In) | wrong |
| models `gemini-3.6-flash`, `gemini-2.5-pro`, `gemini-2.5-flash`, `gemini-2.0-flash-thinking` | `agy models` lists `gemini-3.8-flash-{high,medium,low}`, `gemini-3.7-flash-*`, `gemini-3.6-flash-*`, `gemini-3.1-pro-{high,low}`, `claude-sonnet-4-6`, `claude-opus-4-6-thinking`, `gpt-oss-120b-medium` | wrong |

### The stream-json schema

One JSON object per line, keyed by `event`:

```json
{"event":"init","conversation_id":"<uuid>","init":{"cwd":"…","tools":["run_command","view_file",…],"permission_mode":"request-review"}}
{"event":"step_update","step_update":{"conversation_id":"<uuid>","step_index":0,"state":"DONE","step_type":"user_input"}}
{"event":"step_update","step_update":{"conversation_id":"<uuid>","step_index":2,"state":"DONE","step_type":"agent_response","text_delta":"pong\n","duration_seconds":2.28,"usage":{"input_tokens":18389,"output_tokens":15,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":18404}}}
{"event":"step_update","step_update":{"conversation_id":"<uuid>","step_index":3,"state":"DONE","step_type":"checkpoint","usage":{…}}}
{"event":"result","result":{"conversation_id":"<uuid>","status":"SUCCESS","response":"pong\n","duration_seconds":3.19,"num_turns":1,"usage":{…}}}
```

A failed run ends with `{"event":"result","result":{"status":"ERROR","response":""}}`
and exit code 1, with the reason on stderr.

A run that reads a file and runs a command (with
`--dangerously-skip-permissions`) adds:

```json
{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"agent_response","usage":{…}}}
{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"view_file","tool_info":{"name":"view_file","parameters":{"AbsolutePath":"…\notes.txt"}}}}
{"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"tool","tool_name":"view_file","duration_seconds":2.42,"tool_info":{"name":"view_file","parameters":{…},"output":"4 lines, 17 bytes"}}}
{"event":"step_update","step_update":{"step_index":3,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"echo hello"}}}}
{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{…,"output":"hello
"}}}
{"event":"step_update","step_update":{"step_index":4,"state":"ACTIVE","step_type":"agent_response","text_delta":"The"}}
{"event":"step_update","step_update":{"step_index":4,"state":"ACTIVE","step_type":"agent_response","text_delta":" second"}}
…
{"event":"step_update","step_update":{"step_index":4,"state":"DONE","step_type":"agent_response","text_delta":"
","usage":{…}}}
```

So:

- A tool is one `step_index` seen twice: `ACTIVE` (name and parameters) when
  it starts, `DONE` (plus `output`) when it ends.
- Reply text streams as many `text_delta` pieces on one `step_index`, the
  last carrying `state: DONE` and `usage`. An `agent_response` step can carry
  no text at all (step 1 above, the model planning its tool calls).
- `usage` is per step; `result.usage` is the turn's total.
- Resume: `--conversation <id>` continued the same conversation (the
  `init` event repeats the id). An unknown id is not an error: `agy` warns on
  stderr (`warning: conversation "…" not found`) and starts a new
  conversation with a new id, so the session id must be taken from each
  turn's `init`, never assumed unchanged.

## Change

### 1. Prompt as an argument

The subprocess controller writes every prompt to stdin today
(`blockcontroller/subprocess/argv.rs`, `build_turn_argv`). Add a prompt
delivery mode to `ProviderConfig`, `stdin` (the default, unchanged for every
other provider) or `argv`. For `argv`, the turn's argv ends with
`-p <prompt>`, and nothing is written to stdin.

Two constraints:

- **Command-line length.** Windows caps a command line at 32,767 characters.
  A prompt carrying injected context can exceed that. Reject a longer prompt
  with a clear error rather than truncating it; measure how long real launch
  prompts get before deciding whether a fallback is needed.
- **Run `agy.exe`, not a `.cmd` shim.** Arguments passed through a `.cmd`
  file are re-parsed by `cmd.exe`, so a prompt containing `&`, `|`, `%` or
  `"` could break out of the argument. On this machine PATH finds
  `C:\Users\<user>\bin\agy.cmd`, a one-line shim that calls
  `%LOCALAPPDATA%\agy\bin\agy.exe`. For a provider using `argv` delivery,
  resolution must prefer the real executable (`known_install_paths` first,
  then PATH, accepting `.exe` only) and refuse a `.cmd`/`.bat`.

### 2. Flags

- Launch args: `--output-format stream-json --dangerously-skip-permissions`,
  then `-p <prompt>` from (1).
- Resume: `--conversation <id>`, through the existing `flag` resume strategy.
- Session id field: `conversation_id`.

### 3. A translator for `agy`'s events

A new `styled_output_format`, `agy-stream-json`, with its own translator next
to `gemini-translator.ts`:

- `init`: the session starts, and `conversation_id` becomes the session id.
- `step_update` with `step_type: agent_response`: assistant text from
  `text_delta`; `usage`, if present, feeds the token counters.
- Tool steps: mapped once their shape has been captured (see above).
- `result`: the turn ends; `status: ERROR` shows as a turn error, with the
  stderr text.
- Unknown `step_type` values are ignored, not shown as errors.

The history parser (`parseHistoryLines.ts`) and the live feed must accept the
same format, as they do for the other translators.

### 4. Sign-in

`agy` has no auth subcommand, and it reads neither of the variables AgentMux
set for per-account isolation (`ANTIGRAVITY_CONFIG_DIR`,
`ANTIGRAVITY_FORCE_FILE_STORAGE`; neither string appears in `agy.exe`). It
signs in once per machine. So:

- A new `authType`, `cli-managed`: the CLI owns its sign-in, there is no
  account to bind, and launch is not blocked on choosing one (it was, for
  `oauth`, so no Antigravity agent could launch).
- Check: `agy models`, which needs a signed-in account. Not yet verified
  signed out: if it hangs, the check's timeout makes the launch go ahead
  with a warning, and a turn fails with `agy`'s own message.
- Sign-in: a terminal running plain `agy`, which signs in on first run.

### 5. Models and permission modes

The catalog lists the newest of each family `agy models` prints, defaulting
to `gemini-3.8-flash-medium` (also `default_model_for` in srv), and `--model`
is passed on launch and on every send, so the create modal shows the picker.
A bare Claude alias left in a pane's runtime (`sonnet`) is never passed;
agy's own `claude-sonnet-4-6` is. Reading `agy models` at runtime instead of
a fixed list is a follow-up.

Permission modes map to agy's own flags: bypass is
`--dangerously-skip-permissions`, plan and accept-edits are `--mode plan` /
`--mode accept-edits`, and default and auto add nothing (agy has no auto).
`--effort` exists in agy (low/medium/high) but is not wired.

### 6. Also changed

- **Tool ids** are `agy-<conversation_id>-<step_index>`. Step indexes keep
  counting across a resumed conversation, so the pair is unique, and it comes
  from the line itself, which `readToolResult` re-parses alone.
- **The user's message** is written to the transcript by the subprocess
  controller (`persists_user_record`), as for Codex and Kimi, since `agy`
  doesn't echo it; the live feed's roll-off includes the format.
- **The app API's `agent.open`** stamped every provider outside a hard-coded
  list as `claude-stream-json`. It now writes the provider's own
  `styled_output_format`, as the UI launch does.
- **Containers** refuse a provider that takes its prompt as an argument: the
  container path keeps prompts out of argv on purpose. Antigravity is
  host-only anyway.

### 7. Not changed

- `/btw` side questions and the ambient title/digest calls don't support
  Antigravity; they fail or are refused as before.
- A failed turn is detected from the exit code and stderr, not from
  `result.status`.

## Out of scope

- Gemini CLI. It keeps its own translator and flags; it now needs a Gemini
  API key or a Code Assist licence (#4403).
- Updating `agy`. It updates itself.

## Tests

- `argv.rs`: `argv` delivery puts `-p <prompt>` last and writes nothing to
  stdin; resume adds `--conversation <id>`; an over-long prompt is refused.
- CLI resolution for an `argv` provider never returns a `.cmd` or `.bat` path.
- Translator unit tests from the captured runs, stored in
  `frontend/test/fixtures/providers/agy/1.1.11/` with the local user name and
  temp folder replaced.
- Manual, in a `task dev` build: create an Antigravity agent, send two
  prompts (the second resumes the same conversation), run one that uses a
  tool, and check that the reply, tool call and token counts show.

## Open questions

1. ~~How long are real launch prompts?~~ The startup message carries identity,
   accounts, startup instructions and up to ten peers; memory goes to
   `GEMINI.md`. A longer prompt is refused (30,000 UTF-16 units) with a clear
   message.
2. What does `agy models` do when signed out?
3. ~~The tool-step event shapes.~~ Captured above.
