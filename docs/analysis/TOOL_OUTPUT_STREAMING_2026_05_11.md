# Tool output streaming — analysis and solution options

**Status:** Analysis (decision pending)
**Owner:** AgentA
**Date:** 2026-05-11
**Driving question:** Why doesn't `tool_chunk` "just work" if we wire it in the backend, and what does it actually take to ship live bash output in the agent pane?

---

## 1. TL;DR

The Phase 2 task in [SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md](../specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md) — *"Connect stdout/stderr line streaming on the host side to emit `tool_chunk` events"* — is **not implementable as written**. It assumes Claude Code CLI exposes partial tool output through its `stream-json` protocol. It does not, and the Anthropic Messages API has no public roadmap entry that changes that. Every comparable product (Cursor, Cline, OpenHands, Continue, Zed) ships its **own** command runner outside the model's tool model and streams that runner's stdout to the UI through a side channel.

For AgentMux this is a small architectural shift, not a one-PR feature. The recommended path is an **MCP-mediated runner with an out-of-band stream channel** that reuses our existing WPS broker. Three options are evaluated in §5; the recommendation is option **B+** with an option-A intermediate ship for screenshot value.

---

## 2. What we confirmed, with receipts

### 2.1 Claude Code stream-json does not carry partial tool output

Events present in `--output-format stream-json --include-partial-messages`:

| Event | Payload | Streams partially? |
|---|---|---|
| `content_block_delta` / `text_delta` | assistant prose | Yes |
| `content_block_delta` / `thinking_delta` | reasoning | Yes |
| `content_block_delta` / `input_json_delta` | tool **input** JSON | Yes (fine-grained tool streaming) |
| `tool_use` block (in assistant message) | tool call site | Whole |
| `tool_result` block (in user message) | tool output | **Whole — buffered until command exits** |

No `tool_output_delta` event exists. Fine-grained tool streaming (`eager_input_streaming: true`) streams *inputs*, never outputs. Anthropic's public docs surface this as a design choice, not a roadmap item.

> Sources: `platform.claude.com/docs/.../tool-use/fine-grained-tool-streaming`, `platform.claude.com/docs/.../streaming`.

### 2.2 Hooks cannot synthesize stream events

`PreToolUse` and `PostToolUse` are the only points where AgentMux can interpose. Their JSON contract:

- `PreToolUse` returns `permissionDecision: "allow" | "deny"` and optionally `updatedInput` (rewrites the bash command).
- `PostToolUse` returns optionally `updatedToolOutput` (rewrites the final result before Claude sees it).
- **Neither hook can emit `content_block_delta` events into the stream-json output**. Hook return values flow back to Claude on the *next turn*, not into the live stream.

Practical consequence: a `PreToolUse` hook can route Bash through our own runner, but the result still arrives at AgentMux as one whole `tool_result` block.

### 2.3 The CLI does not expose `--allowed-tools` / `--disallowed-tools`

We currently spawn:

```
claude -p --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions
```

(`agentmux-srv/src/backend/providers.rs:100-107` + `subprocess.rs:250-281`.) No tool-allowlist switch is documented in the CLI today. An MCP-registered `bash` tool will sit *alongside* the native Bash tool; Claude picks one per call based on its own tool-selection heuristics and the tool description we register. We **cannot** force Claude to use our MCP bash by config flag alone — we have to make ours look better (description + system prompt nudge), or block the native one via a `PreToolUse` hook.

### 2.4 AgentMux already injects an MCP server

`agentmux-srv/src/backend/agent_config.rs:230-293` auto-writes an `agentmux` MCP server entry into the agent's working-dir `.mcp.json` with stdio transport. This is the natural place to add a streaming `bash` tool — no new server scaffolding needed.

### 2.5 Every peer product uses host-side interception

| Product | Bash runner | Streaming channel |
|---|---|---|
| **Cline** | VSCode shell-integration API (`terminal.shellIntegration.executeCommand`) | Async for-await over `execution.read()` |
| **Cursor** | Own terminal | SSE / proprietary stream |
| **OpenHands** | `od-runtime-client` inside Docker, `pexpect` on /bin/bash | EventStream → `CommandOutputObservation` events; agent loop replays them on next turn as concatenated tool_result |
| **Continue.dev** | VSCode terminal | Same shell-integration pattern as Cline |
| **Zed** | ACP adapter intercepts before native bash runs | ACP stream events |
| **Codex CLI** | Native tool | Whole result today; PR #13640 (in flight) adds long-lived PTY exec |

Zero of them rely on Anthropic delivering partial tool output. The MCP ecosystem has shell servers (`ripple`, `mcp-shell`, `tumf/mcp-shell-server`, `g0t4/mcp-server-commands`, `stdout-mcp-server`) — only **ripple** streams (named pipes + OSC 633 markers). MCP's request/response model itself doesn't carry partial tool results, which is why competitors stream through their own channels rather than through the protocol.

---

## 3. The real shape of "streaming bash output"

There are two channels in play; conflating them is the source of the Phase 2 confusion in the original spec.

```
                                  ┌─────────────────────────────────┐
                                  │             Claude              │
                                  │  (decides to call bash, waits   │
                                  │   for whole tool_result block)  │
                                  └────────────┬────────────────────┘
                                               │ MCP stdio
                                               │ tool_use → wait → tool_result
                                               ▼
┌──────────────┐     stdout       ┌──────────────────────────────┐
│ The bash     │ ───── line ────► │ AgentMux MCP `bash` runner   │
│ command      │                  │ (lives in agentmux-srv or    │
│ (pty/sh -c)  │                  │  the agentmux-mcp binary)    │
└──────────────┘                  └────┬──────────────────┬──────┘
                                       │                  │
              concatenated final stdout│                  │ per-line WPS
              ← back into Claude as    │                  │ events on a new
                tool_result block      ▼                  ▼ "tool_chunk" subject
                                ┌────────────┐    ┌──────────────────┐
                                │ stream-json│    │ AgentMux         │
                                │  pipe to   │    │ frontend         │
                                │  frontend  │────┤ correlates by    │
                                └────────────┘    │ tool_use_id      │
                                                  └──────────────────┘
```

**Channel 1 (existing):** Claude → AgentMux frontend via stream-json. Carries `tool_call` (with id) and eventually `tool_result` (whole). Frontend already parses both.

**Channel 2 (new):** Our runner → AgentMux frontend via the existing WPS broker. Carries `tool_chunk` events keyed by the same id, line by line, in real time.

The frontend reducer plumbed by [PR #800](https://github.com/agentmuxai/agentmux/pull/800) is already shaped for this: `ToolChunkAppend` takes a `toolId` and a chunk; `StreamFlush` preserves `log.chunks` across the running→terminal transition. The data shape is right; the source of the chunks just isn't Claude's stream-json — it's our runner.

---

## 4. Constraints and non-goals

- **No protocol-level streaming changes.** Anthropic Messages API spec is fixed for our timeline. We don't wait on `tool_output_delta`.
- **No replacement of Claude Code CLI** with Agent SDK embedding. That's a separate, much larger architectural call (touches auth, session resume, partial messages, all provider integrations). Keep it on the "future" list.
- **Provider parity.** Codex and Gemini providers face the same gap. Whatever pattern we pick should generalize — they each get their own runner injection.
- **No regression on `--dangerously-skip-permissions`.** We currently rely on it. If the chosen path forces us off it, that's a separate scope (and intersects [SPEC_DECISION_PROMPT_2026_04_24.md](../specs/SPEC_DECISION_PROMPT_2026_04_24.md)).
- **Frontend reducer is settled.** Anything that ships partial output goes through `ToolChunkAppend` — no parallel state store.

---

## 5. Solution options

Three real options, ordered by cost.

### Option A — Post-hoc synthesis from `tool_result`

When `tool_result` arrives whole, the translator splits the content by newline and dispatches one `ToolChunkAppend` per line. The UI then virtualizes the buffer and renders an action bar at the bottom — same overlay design, same data shape, but the "streaming" is a render trick: every line arrives in the same RAF.

- **Cost:** ~120 LOC in `claude-translator.ts` + per-provider parity (Codex/Gemini translators each get the same arm).
- **Value:** Zero live feedback during the actual run; the user still stares at a spinner for 3-minute commands. Visible "streaming" only happens after the command finishes — anti-feature for `npm install && npm test`.
- **Verdict:** Worth shipping only as a stepping stone to validate the overlay UI (Phase 3) without blocking on the real runner. Otherwise misleading.

### Option B — MCP bash runner with WPS side-channel **(recommended)**

Extend the existing `agentmux-mcp` server with a `bash` (and `shell`) tool whose execution path:

1. Generates / receives a `tool_use_id` (from the MCP request — Claude provides it).
2. Spawns a PTY-backed `bash -c "$command"` (or `sh` / `pwsh` per platform).
3. Streams stdout + stderr line-by-line to a new WPS subject keyed by `tool_use_id`. Each line publishes one `{ kind, content, timestamp }` payload.
4. Buffers the full output internally.
5. Returns the buffered output to Claude as the MCP `tool_result` when the command exits.

Frontend side, the new WPS subject is bridged into `useAgentStream` as synthetic `tool_chunk` events (already plumbed). Dedup is timestamp + last-content based; reducer is unchanged.

To get Claude to actually call our MCP bash instead of native Bash, two paths:

- **(B-soft)** Tool description nudging + system prompt. Our `mcp__agentmux__bash` advertises "live-streaming output, preferred for long-running commands." Claude picks it most of the time — but not always.
- **(B-hard)** A `PreToolUse` hook on native `Bash` that returns `permissionDecision: "deny"` with `permissionDecisionReason: "Use mcp__agentmux__bash instead — streams output to the UI"`. Claude retries via the MCP tool. Deterministic but does add a turn-of-friction on first invocation.

Recommend **B-hard** for the deterministic path: zero non-streaming bash invocations possible.

- **Cost:** ~400-600 LOC across `agentmux-mcp/src/bash.rs` (new), `agentmux-srv` WPS subject wiring, frontend bridge from new WPS subject → `dispatchDoc(ToolChunkAppend)`, `.claude/hooks.json` auto-injection in `agent_config.rs`, plus tests.
- **Value:** Real live streaming. Matches every peer product's pattern. Re-uses our existing infra (WPS broker, MCP injector).
- **Risks:**
  - **Tool-selection drift.** If Claude's tool-selection heuristic flips, B-soft fails silently. B-hard's `PreToolUse` deny safeguards this. Test matrix needs an explicit "Claude picks mcp__agentmux__bash" assertion per provider release.
  - **PTY portability.** `portable_pty` crate works on Win32 ConPTY + Unix; we already depend on it elsewhere. No new dep.
  - **Permission flow.** B-hard's hook interacts with the per-tool permission decision panel ([SPEC_DECISION_PROMPT_2026_04_24.md](../specs/SPEC_DECISION_PROMPT_2026_04_24.md)). Need a sequencing decision: deny native → mcp call gets a fresh permission prompt, or auto-allow on the redirect.
  - **stderr stays out of the model's view if we don't include it.** Concatenated tool_result should include stderr (interleaved with stdout, prefixed) so Claude reasons about errors correctly. Different from frontend rendering, which interleaves by arrival time.
  - **Multi-line dedup.** If a chunk re-arrives during replay (history), reducer dedups against the last chunk only. Long-running tools with hours of output could trip dedup false-positives. Keep the bounded ring (50k lines per [SPEC_TOOL_BLOCK_LIVE_LOG §5](../specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md)).
- **Verdict:** This is the path. Closes the gap fully, generalizes to other providers, and is the smallest delta that actually delivers live streaming.

### Option C — Replace Claude Code CLI with Agent SDK embedding

Drop the CLI subprocess and embed Anthropic's Agent SDK directly in `agentmux-srv`. Implement Bash as a host-side tool with our own streaming. Use the SDK's `permission_callback` for the decision flow.

- **Cost:** Easily 4-6 weeks. Replaces every CLI-mediated codepath: auth, session resume, partial messages, slash commands, all providers' equivalent. Codex/Gemini have no equivalent SDK; we'd be heterogeneous.
- **Value:** Cleanest interception model, full control. Long-term right answer if we go all-in on Anthropic.
- **Verdict:** Out of scope for live-log work. Park as a long-horizon discussion.

---

## 6. Recommendation

**Ship in two PRs:**

1. **PR α (the screenshot):** Phase 3 UI (overlay with header / virtualized log / bottom action bar) wired to whatever `ToolNode.log.chunks` happens to contain, plus option **A** in the Claude translator so we have visible content while option B lands. The render-trick streaming is wrong, but the overlay is right, and we want it in users' hands.
2. **PR β (the real thing):** Option **B-hard**. Adds the streaming MCP bash tool, the WPS bridge, the `PreToolUse` deny hook, and per-provider parity stubs. PR α's translator synthesis gets removed in the same PR — single source of truth for chunks is the runner.

This lets us land the UI value in days, defer the runner work to the right scope, and avoid carrying the post-hoc synthesizer as permanent code.

Provider parity for Codex / Gemini in PR β is bandwidth-bounded — start with Claude, file follow-ups for the others. The MCP bash tool is provider-agnostic on the server side; the only per-provider work is the `PreToolUse`-equivalent in their respective hook systems (or alternate nudge if absent).

---

## 7. Open questions to resolve before PR β

- **PTY vs `Stdio::piped()`?** Real PTYs surface progress bars and ANSI redraws correctly (think `npm install`'s spinner); pipes flatten them. Recommend PTY via `portable_pty`.
- **Output size cap?** Long builds produce 100k+ lines. Frontend caps at 50k per spec; should the MCP runner also truncate what it returns to Claude, or send the whole thing? Truncating in the runner risks the model losing critical end-of-output errors. Probably: full output to Claude (with a head/tail compression for >1MB), capped view to the frontend log.
- **What happens to direct `task dev` / inline terminal Bash calls** that don't route through the agent at all? Out of scope — they live in the terminal pane, separate code path.
- **Per-OS shell selection.** Bash on Linux/macOS; pwsh-then-cmd on Windows (we already detect this in `agentmux-srv/.../shell.rs`). Reuse that path.
- **Hook injection mechanics.** We already write `.claude/hooks.json` from `agent_config.rs:112-119`; the `PreToolUse` redirect entry is one new JSON object. No new infra.
- **Permission flow interaction.** With `--dangerously-skip-permissions` set, the `PreToolUse` deny still fires and re-routes — but if we ever turn permissions back on, the redirect must auto-allow the MCP follow-up. Decision needed alongside [SPEC_DECISION_PROMPT_2026_04_24.md](../specs/SPEC_DECISION_PROMPT_2026_04_24.md).

---

## 8. Cross-references

- Live-log data shape + reducer (PR #800): [SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md](../specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md)
- Permission gating: [SPEC_DECISION_PROMPT_2026_04_24.md](../specs/SPEC_DECISION_PROMPT_2026_04_24.md)
- Existing MCP server injection: `agentmux-srv/src/backend/agent_config.rs:230-293`
- Claude CLI spawn: `agentmux-srv/src/backend/providers.rs:100-107`, `agentmux-srv/src/backend/blockcontroller/subprocess.rs:250-281`
- Subprocess output → WPS: `agentmux-srv/src/backend/blockcontroller/subprocess.rs:442-561` (stdout) + `563-586` (stderr — currently logged only)
- ToolBlock rendering: `frontend/app/view/agent/components/ToolBlock.tsx`
- DocumentRow: `frontend/app/view/agent/virtualization/DocumentRow.tsx:232-238`

## 9. Sources

**Anthropic primary docs:**
- Fine-grained tool streaming — `platform.claude.com/docs/.../fine-grained-tool-streaming`
- Streaming messages — `platform.claude.com/docs/.../streaming`
- Claude Code hooks — `code.claude.com/docs/.../hooks`
- Agent SDK hooks — `code.claude.com/docs/.../agent-sdk/hooks`
- MCP — `code.claude.com/docs/.../mcp`

**Competitive landscape:**
- Cursor terminal — `cursor.com/docs/agent/tools/terminal`
- Cline repo + issue #6708 + #10524 — `github.com/cline/cline`
- OpenHands deep-dive + issue #2404 — `github.com/OpenHands/OpenHands`
- Zed ACP + Claude Code via ACP — `zed.dev/acp`, `zed.dev/blog/claude-code-via-acp`
- Codex issue #4751 + PR #13640 — `github.com/openai/codex`

**MCP shell server prior art:**
- `github.com/yotsuda/ripple` (streaming via named pipes + OSC 633)
- `github.com/amitdeshmukh/stdout-mcp-server`
- `github.com/sonirico/mcp-shell`
- `github.com/tumf/mcp-shell-server`
- `github.com/g0t4/mcp-server-commands`
