# Codex CLI JSONL Adapter Contract

**Date:** 2026-08-08
**Status:** Draft
**Scope:** Codex CLI subprocess output, turn lifecycle, session continuity, translation, and fixtures
**Target:** AgentMux Codex provider (`styledOutputFormat: "codex-json"`)
**Current AgentMux pin:** `@openai/codex@0.116.0`
**Current locally inspected CLI:** `codex-cli 0.147.0`

---

## 1. Purpose

Define the provider boundary between Codex CLI JSONL and AgentMux's provider-neutral
agent stream. This document replaces the Codex output assumptions in
`docs/specs/codex-gemini-cli-integration.md`; that earlier document predates the
documented `codex exec --json` contract.

This is a transport and translation spec. It answers:

1. How AgentMux invokes a new Codex turn and a resumed Codex turn.
2. Which stream owns lifecycle truth.
3. How JSONL events become AgentMux `StreamEvent` values.
4. How duplicate snapshots, unknown events, malformed lines, and process exits behave.
5. Which captured fixtures and acceptance tests are required before changing the
   pinned Codex CLI version.

The goal is a Codex pane that remains correct as the CLI adds fields and event types.
The adapter MUST be strict about invariants that affect AgentMux state and tolerant
about provider data it does not yet understand.

---

## 2. Sources and observed baseline

The normative upstream references are:

- [OpenAI Codex non-interactive mode](https://developers.openai.com/codex/noninteractive)
- [OpenAI Codex CLI command reference](https://developers.openai.com/codex/cli/reference/)

As of 2026-08-08, the official documentation establishes that:

- `codex exec --json` writes one JSON object per line to stdout.
- Top-level event families include `thread.started`, `turn.started`,
  `turn.completed`, `turn.failed`, `item.*`, and `error`.
- Documented item categories include agent messages, reasoning, command executions,
  file changes, MCP tool calls, web searches, and plan updates.
- `thread.started.thread_id` is the resumable session identifier.
- A saved non-interactive session resumes through
  `codex exec resume <SESSION_ID>`.

The local `codex-cli 0.147.0` help additionally confirms that both first-turn and
resume commands accept `--json`, that resume accepts `-` as a stdin prompt, and that
`--dangerously-bypass-approvals-and-sandbox` is intended for an externally sandboxed
environment.

Official documentation defines the public event families but does not freeze every
field on every item object. Therefore:

- captured output from the pinned CLI is the executable schema baseline;
- the upstream docs are the semantic baseline;
- exact item-field assumptions MUST be backed by a committed fixture;
- fixtures MUST record the CLI version that emitted them.

---

## 3. Product decisions and non-goals

### 3.1 Decisions

| Concern | Decision |
|---|---|
| Integration surface | Stable non-interactive CLI: `codex exec --json` |
| Controller | One subprocess per turn |
| Prompt transport | UTF-8 stdin, followed by newline and EOF |
| Session continuity | Persist `thread_id`; resume with the Codex `exec resume` subcommand |
| Execution policy | Keep `--dangerously-bypass-approvals-and-sandbox` as the AgentMux default |
| External isolation | AgentMux's Docker system is the sandbox boundary |
| Stream framing | One independent JSON object per non-empty stdout line |
| Lifecycle authority | `turn.completed` or `turn.failed`; process exit is fallback only |
| Compatibility | Ignore unknown fields; retain and count unknown event/item types |
| Testing | Versioned, redacted JSONL fixtures plus a Docker live-smoke matrix |

### 3.2 Docker/bypass assumption

This spec does not redesign AgentMux's Docker execution system or change its default
permission policy. Codex is intentionally launched with bypass because the provider
process and its descendants execute inside AgentMux's external container boundary.

The implementation MUST NOT silently translate the bypass setting into Codex's
internal `workspace-write` sandbox. That would create two competing sandbox models
and make observed behavior differ by provider. Container admission, mounts, network
policy, credential exposure, and host escape prevention belong to the Docker runtime
contract, not the JSONL adapter.

### 3.3 Non-goals

This spec does not define:

- CLI installation or authentication UI;
- model discovery or the model/effort picker;
- Docker image construction or container security policy;
- the Codex SDK, app-server, MCP-server, TUI, or interactive approval protocol;
- a new generic plan/document node type;
- changes to Kimi, Gemini, Claude, or ACP adapters.

---

## 4. Invocation contract

### 4.1 First turn

Conceptual argv:

```text
codex exec
  --json
  --color never
  --dangerously-bypass-approvals-and-sandbox
  -
```

The implementation passes argv directly; it MUST NOT concatenate a shell command.
Provider/model/config flags may be inserted before the final `-` by the existing
runtime argument builder.

`--ephemeral` MUST NOT be used because AgentMux needs Codex's persisted rollout to
resume the thread on the next subprocess turn.

### 4.2 Resumed turn

Conceptual argv:

```text
codex exec resume
  --json
  --color never
  --dangerously-bypass-approvals-and-sandbox
  <thread_id>
  -
```

Codex resume is a subcommand shape, not a flag/value suffix. AgentMux's existing
`resume_flag` abstraction cannot express it. Implementation MUST introduce an argv
strategy capable of constructing provider-specific first-turn and resumed-turn argv;
it MUST NOT special-case string insertion at an arbitrary index in
`host_spawn.rs` and `container_spawn.rs` separately.

Both host and container subprocess paths MUST call the same pure argv builder and
have table-driven tests that assert byte-for-byte equivalent arguments.

### 4.3 Prompt write

For each turn AgentMux MUST:

1. spawn the subprocess with piped stdin/stdout/stderr;
2. write the exact user payload as UTF-8;
3. append one newline if the payload does not already end in one;
4. flush and close stdin to deliver EOF;
5. read stdout until EOF even after a terminal JSONL event, so the pipe cannot block
   the child during shutdown.

AgentMux MUST NOT place the prompt in argv. This avoids command-line length limits,
quoting differences, and process-list disclosure.

### 4.4 Environment and working directory

- `CODEX_HOME` MUST remain stable for all turns in one bound Codex identity.
- The working directory MUST remain the AgentMux agent/pane working directory.
- `thread_id` persistence is scoped to the AgentMux agent instance/session, not only
  to a transient pane component.
- stdout is protocol data. stderr is diagnostic data and MUST NOT be fed through the
  JSONL translator.

---

## 5. JSONL framing and raw retention

### 5.1 Line rules

For every stdout line:

1. Strip the trailing `\r` from CRLF framing but otherwise preserve bytes.
2. Ignore an empty/whitespace-only line.
3. Parse the line as one JSON object.
4. Reject JSON arrays, scalars, and `null` as malformed protocol records.
5. Dispatch the object using its string `type` field.

The parser MUST process lines incrementally and MUST NOT buffer the full turn before
translation.

### 5.2 Raw source of truth

Every valid JSON object MUST continue through the existing blockfile/transcript path
before or alongside frontend translation. Unknown objects are not errors and MUST not
be discarded from persistent history.

This provides:

- replay after renderer restart;
- retranslation after an adapter update;
- evidence when the CLI schema changes;
- exact bug fixtures without relying on screenshots or logs.

### 5.3 Malformed records

A malformed stdout line MUST:

- increment a `codex_jsonl_malformed_total` diagnostic counter;
- be retained in a bounded diagnostic sample with secrets redacted;
- not crash, reset, or poison the translator;
- not end the turn;
- not be rendered as assistant prose.

If the process exits without `turn.completed` or `turn.failed`, the controller uses
the process-exit fallback in section 8. A malformed line alone is never terminal.

### 5.4 Unknown records

An unknown top-level type or item type MUST:

- be retained in raw history;
- increment a counter keyed by the unknown type;
- produce zero user-visible `StreamEvent` values;
- leave known item state and turn state intact.

Unknown fields on a known type MUST be ignored. The adapter MUST NOT validate known
objects with `additionalProperties: false`.

---

## 6. Stateful item reducer

Codex item events are lifecycle snapshots, not independent chat messages. The
translator MUST retain per-turn state keyed by `item.id`:

```ts
interface CodexItemState {
    type: string;
    lastSnapshot: unknown;
    emittedTextLength: number;
    emittedOutputLength: number;
    toolOpened: boolean;
    terminal: boolean;
}
```

Required behavior:

- `item.started`: create state; open a tool node when enough identity is present.
- `item.updated`: compare with the previous snapshot and emit only the new suffix or
  newly available structured fields.
- `item.completed`: emit any unobserved tail, then finalize the item exactly once.
- repeated snapshots are no-ops;
- an update after completion is retained and counted but not rendered;
- completion without a prior start is valid and synthesizes the missing start;
- item IDs are opaque strings and MUST never be synthesized with time-based values
  when the provider supplies one.

For text-like fields, if the new value begins with the prior value, emit the suffix.
If it does not, treat it as a replacement snapshot: emit no speculative text delta,
retain the replacement for diagnostics, and emit the final full value only if no
content for that item has yet been rendered. This prevents duplicated prose when a
CLI changes snapshot behavior.

The item reducer resets on a new `turn.started`, after terminal turn finalization, and
when the translator is explicitly reset. It does not reset merely because a
`thread.started` record repeats during resume.

---

## 7. Translation matrix

The field names in the table below describe the currently documented/observed Codex
JSONL family. A committed fixture is required before implementation relies on a more
specific field shape.

### 7.1 Top-level events

| Codex event | AgentMux behavior |
|---|---|
| `thread.started` | Validate non-empty `thread_id`; persist it; render nothing |
| `turn.started` | Begin/reset per-turn item state; render nothing |
| `item.started` | Route `item` through the item reducer |
| `item.updated` | Route `item` through the item reducer; emit deltas only |
| `item.completed` | Route `item`; flush its tail and finalize once |
| `turn.completed` | Flush safe pending tails; emit exactly one `session_end` with usage |
| `turn.failed` | Surface structured error; emit exactly one `session_end` |
| `error` | Surface a structured nonterminal error; await turn terminal event or exit |

### 7.2 Item events

| `item.type` | Provider-neutral translation |
|---|---|
| `agent_message` | Text snapshots become `text`; no duplicate on completion |
| `reasoning` | Reasoning snapshots become `thinking`; honor the existing thinking visibility setting |
| `command_execution` | One `tool_call` named `Shell`; output growth becomes `tool_chunk`; completion becomes `tool_result` with status and exit code |
| `file_change` | One `tool_call` named `FileChange`; completion becomes `tool_result` carrying the structured change list/diff supplied by Codex |
| `mcp_tool_call` | One `tool_call` using the provider's server/tool identity; completion becomes `tool_result` with result or error |
| `web_search` | One `tool_call` named `WebSearch`; completion becomes `tool_result` containing query/result metadata available on the item |
| `plan_update` | Render the changed plan as `thinking` for phase 1 |
| `todo_list` | Compatibility alias for plan updates; render as `thinking` for phase 1 |

The adapter MUST use Codex `item.id` as the AgentMux tool ID. For MCP calls, the
display name SHOULD be `<server>.<tool>` when both are available, falling back to the
available component and then `MCP`.

### 7.3 Command execution

On the first command snapshot containing `command`:

```ts
{
    type: "tool_call",
    tool: "Shell",
    id: item.id,
    params: { command: item.command }
}
```

As `aggregated_output` grows, emit only the suffix:

```ts
{
    type: "tool_chunk",
    id: item.id,
    kind: "stdout",
    content: suffix
}
```

On completion, emit one `tool_result`:

- `status: "success"` only when Codex reports completed/success and the exit code is
  absent or zero;
- otherwise `status: "failed"`;
- preserve `exit_code` as `exitCode`;
- preserve final output in `result.output`, subject to the existing output cap;
- do not emit the full output again as a chunk.

If future Codex fixtures distinguish stdout and stderr, the adapter SHOULD emit the
corresponding `tool_chunk.kind`; until then, `aggregated_output` maps to `stdout` and
the raw item remains available for diagnosis.

### 7.4 File changes

File changes are modeled as a tool lifecycle so existing expansion, completion, and
failure UI works without a Codex-only document node. The structured result MUST retain:

- path;
- change kind;
- diff/patch text when supplied;
- provider status/error.

The adapter MUST NOT reread files from disk to reconstruct the result. The JSONL item
is the record of what Codex reported; filesystem state may already have changed again.

### 7.5 Reasoning and final messages

- Reasoning content is never reclassified as final assistant text.
- Agent-message content is never reclassified as thinking.
- Item completion does not end a turn.
- Empty content produces no document node.
- Repeated full snapshots produce no duplicate content.

### 7.6 Usage

For `turn.completed.usage`, map at minimum:

- `input_tokens` -> `SessionStats.input_tokens`;
- `output_tokens` -> `SessionStats.output_tokens`.

`cached_input_tokens` and `reasoning_output_tokens` MUST be retained in raw JSONL and
covered by fixtures. Adding provider-neutral stats fields for them is allowed in the
implementation PR but is not required to satisfy the phase-1 JSONL adapter.

---

## 8. Turn lifecycle and failure semantics

### 8.1 Terminal event rule

Only these events normally finalize a Codex turn:

- `turn.completed` -> successful/complete `session_end`;
- `turn.failed` -> visible failure plus `session_end`.

Neither `agent_message`, `item.completed`, stdout EOF, nor the first top-level `error`
is sufficient by itself while the process is still running.

The translator MUST implement a first-terminal-wins gate. Duplicate terminal events
or replayed terminal lines produce no second `session_end`.

### 8.2 Top-level error

A top-level `error` becomes an `error_result` with:

- a provider code when one is present;
- code `0` when Codex provides only a message;
- the provider message without Markdown decoration.

It is nonterminal because Codex may report an error and then emit `turn.failed`, or
may emit a recoverable error while continuing. The terminal gate prevents the later
failure from duplicating finalization.

### 8.3 Turn failure

`turn.failed` MUST:

1. emit an `error_result` unless the same error was already surfaced;
2. flush no speculative partial replacement snapshots;
3. finalize open tool items as failed when they lack a terminal item event;
4. emit one `session_end` with any available usage.

### 8.4 Process-exit fallback

After stdout has drained:

| Exit condition | Fallback |
|---|---|
| Exit 0 after terminal JSONL | No action |
| Non-zero exit after terminal JSONL | Keep terminal result; attach exit diagnostics only |
| Exit 0 without terminal JSONL | Emit protocol failure and one `session_end` |
| Non-zero exit without terminal JSONL | Emit process/provider failure and one `session_end` |
| Killed by AgentMux | Mark open tools denied/failed according to existing stop semantics; finalize once |

The fallback error includes a bounded stderr tail. It MUST NOT dump environment
variables, auth files, or an unbounded subprocess transcript into the pane.

---

## 9. Thread identity and resume

### 9.1 Capture

On `thread.started`:

- require `thread_id` to be a non-empty string;
- persist it through the existing block/session metadata path;
- use the emitted ID as authoritative;
- avoid a metadata write/broadcast when the value is unchanged.

If a resumed subprocess emits a different thread ID, AgentMux MUST log the mismatch,
adopt the newly emitted ID only after the event parses successfully, and persist it.
The new emitted ID is authoritative because it describes the stream currently being
rendered.

### 9.2 Resume failure

If `codex exec resume <thread_id>` fails before a valid `thread.started` event:

- surface the failure;
- mark the stored resume ID stale using the existing session-recovery mechanism;
- do not automatically replay the same user message into a fresh thread;
- allow the user to explicitly start a new session and resend.

Automatic fresh retry is prohibited because the resumed turn may have executed tools
before the local failure became visible, creating duplicate side effects.

### 9.3 Ownership

AgentMux's existing session lease remains required. Two AgentMux processes MUST NOT
send turns concurrently to the same Codex `thread_id`, including one host-spawned and
one container-spawned process.

---

## 10. Fixtures and compatibility gates

### 10.1 Location

Commit redacted fixtures under:

```text
frontend/test/fixtures/providers/codex/<cli-version>/
```

Each scenario contains:

```text
<scenario>.jsonl
<scenario>.manifest.json
```

Manifest fields:

```json
{
  "provider": "codex",
  "cli_version": "0.147.0",
  "platform": "windows-x64",
  "container": true,
  "scenario": "command-success",
  "captured_at": "2026-08-08T00:00:00Z",
  "invocation_shape": "exec --json --color never --dangerously-bypass-approvals-and-sandbox -",
  "redactions": ["thread_id", "absolute_paths", "usernames", "tokens"]
}
```

Synthetic fixtures are allowed only for malformed/unknown compatibility cases and
MUST declare `"synthetic": true`. Normal protocol fixtures MUST come from the pinned
CLI and declare whether they were captured on the host or in Docker. The bootstrap
translator suite may use host captures produced without bypass in a disposable
workspace; the integrated acceptance gate and every pin change still require the
Docker smoke subset captured inside the AgentMux Docker system.

### 10.2 Required scenarios

Before the JSONL implementation is complete, fixtures MUST cover:

1. final answer with no tools;
2. reasoning followed by a final answer;
3. successful command with at least one update before completion;
4. failed command with non-zero exit;
5. file change;
6. successful MCP call;
7. failed MCP call;
8. web search;
9. plan/todo update;
10. top-level recoverable or standalone error;
11. `turn.failed`;
12. two-turn session with explicit resume by captured `thread_id`;
13. stop/cancellation during a running command;
14. unknown top-level event and unknown item type;
15. unknown fields on known events;
16. CRLF input;
17. malformed line followed by valid JSONL;
18. EOF without a terminal turn event;
19. output large enough to exercise the existing cap.

If a scenario cannot be induced reliably, its fixture may be captured from a real
incident or upstream regression. It MUST NOT be guessed from undocumented field names.

### 10.3 Version gate

Changing `pinnedVersion` for Codex requires:

1. capture the required smoke subset with the candidate CLI;
2. replay all older fixtures against the new translator;
3. replay candidate fixtures;
4. review unknown-event/item counters;
5. update the version matrix in this spec or a generated fixture index;
6. pass a two-turn Docker live smoke.

Runtime versions outside the fixture matrix SHOULD produce a diagnostic warning but
MUST NOT be blocked solely for being newer.

---

## 11. Test requirements

### 11.1 Pure translator tests

For each fixture, assert:

- ordered `StreamEvent` output;
- exactly-once text/thinking content;
- exactly one tool open and one tool result per completed tool item;
- output updates become suffix chunks, not repeated snapshots;
- exactly one `session_end`;
- usage mapping;
- no exception on unknown fields/types;
- `reset()` clears item and terminal state.

Tests SHOULD feed the fixture both one line at a time and through randomized chunk
boundaries at the line-framing layer.

### 11.2 Argv tests

Table-driven Rust tests MUST cover first and resumed argv for both host and container
spawn paths. They MUST prove that:

- first turn contains `exec` but not `resume`;
- resumed turn contains `exec resume <thread_id>` in valid order;
- `--json`, bypass, color, model, and provider flags survive resume;
- the final prompt sentinel is `-`;
- spaces, Unicode, and shell metacharacters in thread ID rejection tests never become
  shell syntax;
- no generic `--resume <id>` is generated for Codex.

### 11.3 Controller tests

Controller tests MUST cover:

- capture and debounced persistence of `thread_id`;
- reattach hydration before argv construction;
- resume-ID replacement when Codex emits a new authoritative ID;
- no automatic fresh retry after resume failure;
- process-exit fallback with and without a JSONL terminal event;
- session lease enforcement for host and container turns.

### 11.4 Docker live smoke

The minimum live smoke is:

1. start a clean Codex agent container with a bound test identity;
2. send a prompt that writes a uniquely named file and reports its name;
3. assert command/file-change/final-message rendering;
4. capture the thread ID;
5. start the next subprocess with explicit resume;
6. ask Codex to identify the prior turn's filename without restating it;
7. assert the same thread is used and the answer is correct;
8. stop the container and assert the process tree is reaped.

The smoke MUST run only in the designated test workspace and MUST clean up through the
Docker test harness, never by killing AgentMux processes by image name.

---

## 12. Implementation slices

### Slice A — fixtures and translator

- Capture the current pinned and candidate-version fixture subset.
- Replace the old `function_call`-centric Codex translator with the item reducer.
- Add JSONL framing, unknown/malformed diagnostics, and replay tests.
- Do not change session argv in this slice.

### Slice B — provider argv strategy and resume

- Introduce one shared, pure first/resume argv builder.
- Use it from host and container subprocess paths.
- Persist and resume by `thread_id`.
- Add stale-resume behavior and controller tests.

### Slice C — live Docker verification

- Run the required live smoke.
- Capture final fixtures from the exact pinned candidate.
- Update Codex's pin only if the candidate passes the compatibility gate.

Each slice should be independently reviewable. No slice should refactor unrelated
Claude, Kimi, Gemini, or ACP translation behavior.

---

## 13. Acceptance criteria

The Codex JSONL contract is implemented when:

- a Codex pane renders final text, reasoning, commands, file changes, MCP calls, web
  searches, and plans from captured JSONL without exposing raw protocol lines;
- snapshot updates do not duplicate output;
- command output appears incrementally;
- every normal, failed, stopped, or truncated turn leaves the pane out of `Working`;
- terminal turn state is emitted exactly once;
- a second pane turn resumes the captured Codex thread through `exec resume`;
- closing and reopening the pane preserves resumability;
- host and container spawn paths produce equivalent provider argv;
- unknown future fields and event types do not break known output;
- malformed JSONL cannot crash or permanently poison the pane;
- all required fixture tests and the two-turn Docker smoke pass;
- bypass remains the default execution policy under the external Docker boundary.

---

## 14. Open questions deferred to implementation evidence

1. Does the pinned `0.116.0` CLI emit the same item taxonomy as locally inspected
   `0.147.0`, or should AgentMux move the pin before implementing the translator?
2. What exact item type and fields represent current plan updates (`plan_update`,
   `todo_list`, or another versioned shape)? The captured fixture decides.
3. Does command output distinguish stdout/stderr in current fixtures?
4. Which structured fields are present for web-search results versus only its query?
5. Should cached input and reasoning token counts become first-class `SessionStats`
   fields in Slice A or a separate provider-neutral stats change?

None of these questions blocks drafting the adapter. They block relying on an exact
undocumented field shape without a fixture.
