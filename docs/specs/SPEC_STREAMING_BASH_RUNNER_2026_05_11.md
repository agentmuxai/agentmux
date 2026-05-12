# Streaming bash runner — PreToolUse command rewrite

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-11
**Replaces:** Phase 2 of [SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md](./SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md) ("Connect stdout/stderr line streaming on the host side"), which was unimplementable as written.
**Companion:** [docs/analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md](../analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md) — option analysis that ruled in this approach.

**Implementation update (2026-05-11):** Renamed the wrapper from `agentmux-mcp` (the originally-assumed pre-existing crate that turns out NOT to exist in this workspace) to `agentmux-bashwrap` — a new Rust binary in this workspace. Subcommands renamed from `bash-wrap` / `hook-pretooluse-bash` to `exec` / `hook` for cleaner UX. The wrapper crate landed in PR β.A; wire-up (HTTP publish endpoint, env re-injection, hook auto-injection, frontend bridge) will follow in PR β.B.

---

## 1. Goal

Stream Bash stdout/stderr to the agent pane in real time, with full PTY fidelity (ANSI/colors, spinners, interactive prompts), while keeping the `claude` CLI subprocess as the agent runtime so subscription-tier OAuth keeps working. Make `cmd1 && cmd2 && long_build` show progress as it actually progresses — not a 3-minute spinner followed by a wall of text.

Phase 1 ([PR #800](https://github.com/agentmuxai/agentmux/pull/800)) shipped the data shape and reducer (`ToolNode.log.chunks`, `ToolChunkAppend` command, parser arm). This spec is the chunk source.

---

## 2. Why this design

The Claude CLI runs its native Bash tool internally and only surfaces a whole `tool_result` block to us — it does not emit partial tool output in stream-json. We can't fix that from the outside. So we replace the bash *runner*: we own the process that runs the user's command, stream its stdout to the frontend through our existing WPS broker, and hand the buffered result back to the CLI as if its native Bash tool had run normally.

The seam where we interpose is Claude Code's `PreToolUse` hook. Two routes were considered:

| | How it works | Risk |
|---|---|---|
| **Deny-and-redirect** (rejected) | Hook returns `permissionDecision: "deny"` with reason "use mcp__agentmux__bash"; Claude is expected to retry via the MCP tool | Empirical — Claude's reaction to programmatic deny isn't documented. Could be "retry", could be "give up", could be "ask user". One unknown turn of friction *or* a hard fail per session. |
| **Command rewrite** (chosen) | Hook returns `permissionDecision: "allow"` with `updatedInput.command` rewritten to invoke our wrapper binary inline | None of substance — `updatedInput` is part of the documented hook contract and is what Anthropic's docs use as the canonical example |

Command rewrite wins because:

- **No behavioral assumption about the model.** Claude sees a normal allow with a different command; it runs that command and gets a result. No retry loop, no extra round-trip, no model-side decisioning.
- **No MCP tool registration drama.** Claude doesn't need to "choose" our tool over native Bash — there's only one tool path, and we've intercepted what runs inside it.
- **Provider parity is uniform.** Every provider that supports a `PreToolUse`-shaped hook (Claude Code today; Codex/Gemini equivalents as they emerge) plugs in with the same rewrite shape. No per-provider "does the model honor deny-redirect?" matrix.
- **Native Bash semantics preserved.** `cwd`, env, exit code, signal propagation, timeout — all flow through Claude's existing Bash machinery. We only change *what* runs, not *how* the model invokes it.

The pipe-vs-PTY boundary moves: Claude's native Bash spawns our wrapper on a pipe (no PTY), but the wrapper allocates its own PTY for the user's actual command. Pipe is fine for the wrapper's own stdout (it's just text bytes); PTY is critical for the inner command (ANSI/spinners/interactivity).

---

## 3. Architecture

### 3.1 Two channels, correlated by `tool_use_id`

```
                          ┌────────────────────────────────┐
                          │             Claude              │
                          │ (stream-json over stdio,        │
                          │  emits tool_use(Bash, id=X))    │
                          └──────────┬─────────────────────┘
                                     │ tool_use(Bash, id=X, input={command: "..."})
                                     ▼
                          ┌────────────────────────────────┐
                          │  PreToolUse hook fires          │
                          │  (agentmux-bashwrap hook subcommand) │
                          │  • read tool_input.command      │
                          │  • base64-encode it             │
                          │  • return allow + updatedInput  │
                          │    .command = "agentmux-bashwrap     │
                          │      exec                  │
                          │      --tool-id=X                │
                          │      --b64-cmd=<encoded>"       │
                          └──────────┬─────────────────────┘
                                     │
                                     ▼
                          ┌────────────────────────────────┐
                          │  Claude's native Bash tool      │
                          │  bash -c "agentmux-bashwrap          │
                          │    exec --tool-id=X        │
                          │    --b64-cmd=..."               │
                          └──────────┬─────────────────────┘
                                     │ spawns
                                     ▼
            ┌──────────────────────────────────────────────────┐
            │  agentmux-bashwrap exec (subcommand)             │
            │  ├── decode --b64-cmd → original command         │
            │  ├── allocate PTY (portable_pty)                 │
            │  ├── spawn shell -c "$original" inside the PTY   │
            │  ├── read PTY master byte-by-byte                │
            │  ├── line-split + ANSI-aware partial-flush       │
            │  ├── publish each chunk to WPS subject           │
            │  │   "tool_chunk:X" with X-AuthKey               │
            │  ├── buffer everything for the model-visible blob│
            │  └── on exit, write the aggregated blob to stdout│
            └──────────┬────────────────────────────────┬──────┘
                       │ aggregated stdout              │ per-line chunks
                       │ (captured by Claude's bash)    │ keyed by tool_use_id
                       │                                │ (real time)
                       ▼                                ▼
            ┌──────────────────┐            ┌──────────────────┐
            │  Claude →        │            │  WPS broker      │
            │  stream-json     │            │  → frontend      │
            │  → user msg      │            │  useAgentStream  │
            │  → tool_result   │            │  → dispatchDoc   │
            │  → frontend      │            │  (ToolChunkAppend│
            │  reducer         │            │   reducer)       │
            │  (StreamFlush —  │            │                  │
            │  preserves log)  │            │                  │
            └──────────────────┘            └──────────────────┘
```

**Channel 1 (existing):** Claude → CLI → AgentMux stream-json pipe. Carries `tool_use` (Bash + the rewritten command) and eventually `tool_result` (whole aggregated output). Reducer's `StreamFlush` already preserves `log.chunks` when the running ToolNode gets replaced by a terminal-status one (verified by tests in PR #800).

**Channel 2 (new):** Our wrapper → WPS subject `tool_chunk:<id>` → frontend → `dispatchDoc(ToolChunkAppend)`. Carries the live byte stream, line-by-line, while the inner command is running.

Both channels reference the same `tool_use_id`, so the frontend reducer naturally merges them: the running node gets chunks from channel 2 in real time, then channel 1 lands the terminal `tool_result` and the reducer flips `log.open = false` without touching `log.chunks`.

### 3.2 Component breakdown

| Component | Where | What |
|---|---|---|
| **`agentmux-bashwrap exec`** | New subcommand of the existing `agentmux-bashwrap` binary | Inline command runner; allocates PTY; streams to WPS; prints aggregated result to its own stdout |
| **`agentmux-bashwrap hook`** | New subcommand of the same binary | Reads PreToolUse JSON on stdin; emits the `updatedInput` rewrite response |
| **`.claude/hooks.json` auto-injection** | `agentmux-srv/src/backend/agent_config.rs` | Writes the `PreToolUse` matcher on `Bash` pointing at the hook subcommand |
| **WPS subject schema** | `agentmux-common/src/wps.rs` (or wherever subject naming lives) | New subject `tool_chunk:<id>`; payload shape defined here |
| **WPS publish auth** | `agentmux-bashwrap` (both subcommands) | Reads `AGENTMUX_AUTH_KEY` from env (set by the parent claude spawn); attaches `X-AuthKey` on HTTP publish to the sidecar |
| **Frontend bridge** | `frontend/app/view/agent/useAgentStream.ts` | Subscribes to chunk subjects per active tool; dispatches `ToolChunkAppend` |
| **Reducer** | `frontend/app/store/agent-document/reducer.ts` | Tiny shape change to accept multi-chunk batches (RAF coalescing). PR #800's logic absorbs unchanged. |

---

## 4. The bash wrapper subcommand

### 4.1 Invocation shape

The hook rewrites Claude's command from:

```
npm install && npm test
```

to:

```
agentmux-bashwrap exec --tool-id=toolu_abc123 --b64-cmd=bnBtIGluc3RhbGwgJiYgbnBtIHRlc3Q=
```

`--b64-cmd` (URL-safe base64) eliminates every quoting and escaping concern: arbitrary commands, multi-line scripts, embedded quotes, embedded newlines, shell metacharacters, Windows paths — all become opaque ASCII inside the b64.

The wrapper finds `agentmux-bashwrap` on PATH (we already plumb that — it's the MCP server binary Claude already invokes). On Win32 the binary is `agentmux-bashwrap.exe`; the hook emits the platform-correct invocation.

### 4.2 Execution flow

```rust
async fn bash_wrap_main(args: BashWrapArgs) -> Result<i32> {
    let command = base64::decode(&args.b64_cmd)?;
    let command = String::from_utf8(command)?;
    let tool_id = args.tool_id;

    // Locate the sidecar — read endpoint + auth key from env (set
    // by the AgentMux spawn of claude, and inherited through Bash).
    let endpoint = env::var("AGENTMUX_LOCAL_URL")?;  // set by agentmux-srv main.rs:498 — inherited by every spawned child including the wrapper
    let auth_key = env::var("AGENTMUX_AUTH_KEY")?;          // existing

    let publisher = WpsHttpPublisher::new(&endpoint, &auth_key);
    let subject = format!("tool_chunk:{}", tool_id);

    let pty = portable_pty::native_pty_system()
        .openpty(PtySize { rows: 24, cols: 200, ..Default::default() })?;

    let mut cmd = CommandBuilder::new(shell_for_platform());
    cmd.arg("-c").arg(&command);
    cmd.cwd(env::current_dir()?);
    cmd.env("TERM", "xterm-256color");
    // Inherit AGENTMUX_* env so any nested tool that wants the auth
    // key can ask for it.

    let child = pty.slave.spawn_command(cmd)?;
    drop(pty.slave);

    let mut buffered = Vec::<u8>::with_capacity(64 * 1024);
    let reader_handle = spawn_pty_reader(pty.master, publisher, subject, &mut buffered, tool_id);

    let status = child.wait()?;
    reader_handle.await?;

    publisher.publish_terminal(&subject, status.exit_code).await?;

    // Stdout to Claude's Bash subprocess: the aggregated, formatted blob.
    let model_blob = format_for_model(&buffered, status);
    println!("{}", model_blob);

    Ok(status.exit_code)
}
```

### 4.3 Wire format on `tool_chunk:<id>`

```typescript
type ToolChunkMessage = {
    op: "chunk";
    kind: "stdout" | "stderr" | "system";
    content: string;     // UTF-8 decoded; may not end in \n if partial
    timestamp: number;   // unix ms; the source-of-truth dedup key
};

type ToolChunkTerminalMessage = {
    op: "terminal";
    exit_code: number;
    timestamp: number;
};

type ToolChunkPayload = ToolChunkMessage | ToolChunkTerminalMessage;
```

`kind: "system"` is reserved for runner-injected lines (*"Command timed out after 600s"*, *"PTY allocation failed, falling back to pipe"*, etc.). Differentiated in the overlay UI.

PTY merges stdout and stderr by default. To distinguish, we'd need pseudoterminal trickery that's not worth the complexity for v1 — emit everything as `stdout` initially; expose stderr separation as a follow-up via dup'd file descriptors plumbed through the wrapper's own pipes.

### 4.4 PTY behavior decisions

- **Real PTY**, not `Stdio::piped()`. ANSI escapes, terminal-aware progress bars, and interactive prompts all require it. `portable_pty` (already a transitive dep through our terminal path) covers ConPTY + Unix.
- **Size: 24 rows × 200 cols.** Many CLIs format based on `COLUMNS`. 200 is wide enough to avoid premature wrapping in build output without being so wide that `top`-style fullscreen apps misbehave.
- **`TERM=xterm-256color`** in env. Most tools emit color when they detect a 256-color TERM.
- **Byte-level read** + line-splitter in front of WPS publish. Don't wait for `\n` longer than ~50ms — flush partial lines (carriage-return-overwritten progress bars: `npm install`'s `[==>] 50%` updates in place; we emit the latest content and rely on the frontend's CR-handling to overwrite rather than append).
- **Backpressure**: WPS publisher is async + non-blocking. If the sidecar backs up, the wrapper's send queue applies back-pressure to the reader, which is fine — the PTY itself buffers in the kernel until the reader catches up.

### 4.5 Aggregated stdout for Claude

The wrapper's own stdout becomes Claude's `tool_result.content`. Format it so Claude sees both stdout and stderr (interleaved by arrival, prefix-tagged), exit code, and duration:

```
<exited 0 in 3.21s>
$ npm install
added 421 packages in 8s
$ npm test
PASS test/foo.test.ts
  ✓ does the thing (45ms)
Tests: 12 passed, 12 total
```

Stderr lines get prefixed with `[stderr] ` when interleaved into the model-visible blob; the frontend overlay renders them in dim red without the prefix (it has the structural `kind` field from the WPS chunk). Truncate at 50KB head + 50KB tail with `... [N lines elided] ...` in the middle for the model-visible blob; the frontend has the full thing in `log.chunks`.

### 4.6 Concurrency

Multiple tool calls can run concurrently. Each is a separate wrapper process (separate Bash invocation by Claude), so isolation is trivial — separate PTYs, separate WPS subjects keyed by `tool_use_id`, no shared mutable state. Sidecar HTTP can handle parallel publishes without ordering issues (chunks within a subject preserve order by timestamp).

### 4.7 Cancellation

If the user cancels the agent (or Claude itself terminates), Claude's Bash subprocess receives SIGTERM/SIGKILL, which propagates to our wrapper, which:
1. Sends SIGINT to the PTY child (`GenerateConsoleCtrlEvent(CTRL_C_EVENT)` on Win32).
2. Waits up to 2s for graceful shutdown.
3. SIGKILL / `TerminateJobObject`.
4. Publishes a terminal marker with `exit_code: -1`.
5. Exits non-zero.

Claude itself never sees the wrapper exit — Claude was terminated upstream.

---

## 5. The `PreToolUse` hook

### 5.1 Hook entry

Auto-injected by `agentmux-srv/src/backend/agent_config.rs` (existing hook-writing path at lines 112-119) into `<agent_cwd>/.claude/hooks.json`:

```json
{
  "PreToolUse": [
    {
      "matcher": "^Bash$",
      "hooks": [
        {
          "type": "command",
          "command": "agentmux-bashwrap hook"
        }
      ]
    }
  ]
}
```

### 5.2 Hook implementation

```rust
async fn hook_pretooluse_bash_main() -> Result<()> {
    // Hook receives PreToolUse JSON on stdin.
    let input: PreToolUseInput = serde_json::from_reader(std::io::stdin())?;

    if input.tool_name != "Bash" {
        // Defensive — matcher narrows to Bash but be safe.
        emit_passthrough();
        return Ok(());
    }

    let tool_id = &input.tool_use_id;
    let command: &str = input.tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let b64 = base64::URL_SAFE_NO_PAD.encode(command.as_bytes());
    let wrapper = wrapper_invocation(tool_id, &b64);

    let response = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {
                "command": wrapper
            }
        }
    });

    println!("{}", response);
    Ok(())
}

fn wrapper_invocation(tool_id: &str, b64: &str) -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "agentmux-bashwrap".into());
    format!("{} exec --tool-id={} --b64-cmd={}",
        shell_quote(&exe), shell_quote(tool_id), b64)
}
```

`shell_quote` wraps in single quotes (Unix) or `"` (Win32 cmd) to survive whatever shell Claude's Bash tool uses internally (`bash -c "..."` on Unix; `cmd.exe /c "..."` on Windows ConPTY).

### 5.3 Merging with user-provided hooks

`agent_config.rs:112-119` already merges user hooks from `content_map["hooks"]`. Our redirect entry is *appended* to any existing `PreToolUse` array. Hook execution order in Claude Code runs matchers sequentially; if a user has their own `Bash` matcher that emits a deny, ours never runs (the deny wins). If their matcher emits allow, ours fires next and rewrites — which is the desired layering (user policy first, transparent streaming second).

Document this layering behavior in the user-facing hooks docs.

### 5.4 Hook idempotence

The hook can fire twice for the same tool call if the user's hook chain ends with another `allow` that doesn't change `updatedInput.command`. Our rewrite detects an already-wrapped command (`agentmux-bashwrap exec` prefix) and is a no-op in that case to avoid double-wrapping.

---

## 6. Frontend bridge

### 6.1 Subscription lifecycle

`useAgentStream` already iterates `streamEvents`. When a `tool_use` event arrives for `Bash` (the only streaming-aware tool in v1), we open a WPS subscription on `tool_chunk:<tool_use_id>` and keep it open until either:
- An `op: "terminal"` message arrives, or
- The matching `tool_result` lands via stream-json.

```typescript
// In useAgentStream.ts:
if (event.type === "tool_call" && isStreamingTool(event.tool)) {
    chunkSubs.set(event.id, openChunkSubscription(event.id, blockId));
}
if (event.type === "tool_result") {
    const sub = chunkSubs.get(event.id);
    if (sub) { sub.close(); chunkSubs.delete(event.id); }
}
```

`openChunkSubscription` reads from the WPS subject; each `op: "chunk"` message dispatches:

```typescript
dispatchDoc(blockId, {
    type: "ToolChunkAppend",
    toolId,
    chunks: [{ kind, content, timestamp }],   // multi-chunk shape — see §6.3
});
```

For `op: "terminal"` it dispatches a synthesized chunk with `kind: "system"` (e.g. `"[exited 0]"`) and closes the subscription early as a defense against a missing tool_result.

### 6.2 RAF coalescing

Chunks fire at high frequency. The reducer is fast but per-dispatch overhead (audit ring entry, subscriber fanout) adds up at 5000 lines/sec. Coalesce within `useAgentStream`'s existing pendingNew/pendingUpdates → `scheduleFlush()` RAF batching: chunks pile up in a per-tool buffer, flushed once per frame as one `ToolChunkAppend` per affected tool with the accumulated batch.

### 6.3 Reducer shape change (small)

```typescript
// Before (PR #800):
| { type: "ToolChunkAppend"; toolId: string; chunk: ToolLogChunk }

// After:
| { type: "ToolChunkAppend"; toolId: string; chunks: ToolLogChunk[] }
```

Reducer iterates `chunks`, applies per-chunk dedup, emits one audit event with the batched count. PR #800's existing tests update to use the array form; new tests for multi-chunk batches.

---

## 7. Security threading

[PR #801](https://github.com/agentmuxai/agentmux/pull/801) put `/agentmux/reactive/*` (and all backend HTTP) behind `auth_middleware`. The wrapper publishes WPS chunks via HTTP to the sidecar, so it must carry `X-AuthKey`.

Wiring:

- AgentMux's spawn of `claude` already sets `AGENTMUX_AUTH_KEY` in Claude's env (existing). That env is inherited by Claude's child Bash subprocess, and by our wrapper inside it.
- The wrapper reads `AGENTMUX_AUTH_KEY` once at startup. Attaches `X-AuthKey: <key>` to every WPS publish HTTP call.
- The `PreToolUse` hook subcommand also inherits the env. Hook output goes back to Claude over stdout — no HTTP traffic — so no auth header needed there.
- If `AGENTMUX_AUTH_KEY` is missing (unexpected), wrapper logs a system-kind chunk *"Auth key not found — streaming disabled. Output buffered, will appear on completion."*, skips the streaming publish, and still returns the aggregated blob to Claude. Graceful degradation.

For the WPS subject on the frontend side, the existing WPS subscription path already carries auth — no change needed there.

---

## 8. Phasing

Four landable PRs after PR #800:

### PR α — frontend overlay UI ([Phase 3 of the live-log spec](./SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md))
- New `ToolBlockOverlay` + `ToolOverlayLog` (virtualized) + `ToolOverlayActions` (bottom bar).
- Reads `node.log.chunks` (empty today — chunks haven't started flowing).
- Ships visible UI; can be demoed against a hand-dispatched chunk in a unit test.
- Independent of this spec.

### PR β — backend runner (this spec, core)
- New `agentmux-bashwrap/src/bash_wrap.rs` with PTY-backed runner + WPS publishing + X-AuthKey.
- New `agentmux-bashwrap/src/hook.rs` for the `hook` subcommand.
- `agentmux-bashwrap/src/main.rs` adds two subcommands to the dispatch table.
- `agentmux-srv/src/backend/agent_config.rs` extended to write the `PreToolUse` entry.
- `frontend/app/view/agent/useAgentStream.ts` chunk subscription lifecycle.
- Reducer change to multi-chunk `ToolChunkAppend` + test extension.

### PR γ — RAF coalescing + perf
- Per-tool pending chunk buffer flushed at RAF.
- Audit ring sampling for chunk-class commands (1/N or off).
- ANSI parsing offloaded to a worker (or inline if perf is fine).

### PR δ — provider parity (Codex + Gemini)
- Codex CLI hook contract analyzed; equivalent rewrite implemented.
- Gemini provider's hook surface (or its absence) drives whether rewrite is hook-based or wrapper-binary-on-PATH as a fallback.
- File as separate work item; Claude-only first.

---

## 9. Test plan

- [ ] **Basic streaming.** Run `mcp__agentmux__bash`... wait, that's wrong — Claude calls native Bash. Test: agent prompt "run `sleep 2 && echo a && sleep 2 && echo b`". Frontend log shows `a` at t≈2s, `b` at t≈4s, terminal marker at t≈4s. Reducer's chunks array contains both lines in order.
- [ ] **Hook rewrites the command.** Inspect Claude's stream-json — the `tool_use.input.command` field is the wrapper invocation, not the original command. (Sanity check the hook fires.)
- [ ] **ANSI fidelity.** `npm install` colored progress: ANSI escapes preserved in WPS payloads; overlay parses + renders colored.
- [ ] **High-throughput.** `python -c 'import sys; sys.stdout.write("x" * 50000); sys.stdout.flush()'` — 50KB delivered in chunks, no drops, no buffering past 50ms.
- [ ] **Embedded quotes / multi-line.** Agent prompt "run `echo "hello world" && echo 'it works'`". Base64 round-trip preserves it; wrapper executes correctly; no shell-injection regressions.
- [ ] **Interactive prompt.** `read -p "continue? " yn` — verify backend waits; verify frontend overlay shows the prompt line. (Stdin reverse channel out of scope for PR β; tracked as PR γ+. PR β: command blocks until timeout, then SIGINT.)
- [ ] **User-provided hook merge.** `content_map["hooks"]` with a `PreToolUse.Bash` entry → both user hook and our rewrite appear in `.claude/hooks.json` in that order; user-deny short-circuits.
- [ ] **No double-wrap.** If our wrapper's command happens to be re-invoked through the hook (some testing scenario), idempotence prevents `agentmux-bashwrap exec ... agentmux-bashwrap exec ...`.
- [ ] **Cancellation soft.** Agent stop → SIGINT to bash child → graceful exit within 2s → no orphan processes.
- [ ] **Cancellation hard.** SIGKILL path after 2s timeout.
- [ ] **Concurrent tools.** Three concurrent shell commands → three independent WPS subjects fan out correctly; frontend reducer applies chunks to the right tool nodes.
- [ ] **Reconnect mid-stream.** Kill frontend, restart, replay; chunks deduped on (timestamp, kind, content); no duplicates in `log.chunks`.
- [ ] **StreamFlush + log preservation.** Existing PR #800 tests still pass + new test using real WPS chunks instead of synthetic ones.
- [ ] **Cross-platform.** Same scenarios on Win32 (ConPTY) and Linux (Unix PTY) and macOS.
- [ ] **Perf.** 5000 lines/sec sustained for 10s → frontend stays at 60fps (verify with the perf-probe diag from Phase 3).
- [ ] **Auth threading.** Wrapper picks up `AGENTMUX_AUTH_KEY` from env; WPS publishes include `X-AuthKey`; missing key falls back to buffered-only mode with a system chunk.
- [ ] **No auth-key regression.** Subscription-tier OAuth flow unchanged; `--dangerously-skip-permissions` unchanged; existing reactive routes still gated correctly.

---

## 10. Edge cases & risks

### 10.1 Claude internally bypasses the hook for some Bash variant

If Anthropic renames the Bash tool or adds variants (`Bash`, `BashSandbox`, `Shell`), our `matcher: "^Bash$"` misses. Cheap mitigation: matcher uses a broader pattern (`^(Bash|Shell|.*[Bb]ash.*)$`) and a smoke test in CI calls each known shell-tool name.

### 10.2 The wrapper binary isn't on PATH

If `agentmux-bashwrap` isn't on PATH for Claude's bash subprocess, the wrapper invocation fails with `command not found`. AgentMux already installs `agentmux-bashwrap` on PATH for Claude's spawn (existing MCP wiring); verify in CI.

### 10.3 Base64 inflation breaks command-length limits

Bash `argv` has practical limits (~128KB on Linux; smaller on Win32 cmd.exe ~32KB). Base64 inflation is 4/3, so a 24KB command becomes 32KB encoded — uncomfortable on Win32. Mitigation: if the encoded command exceeds 16KB, write the command to a temp file `~/.agentmux/cmd-<id>.sh` and pass `--cmd-file=...` to the wrapper instead.

### 10.4 PTY allocation fails

Fallback to `Stdio::piped()`. ANSI fidelity degrades; spinners freeze. Surface a system-kind chunk: *"PTY unavailable, output may not render correctly."* — same recovery as if `AGENTMUX_AUTH_KEY` is missing.

### 10.5 Hook script crashes / panics

If our `agentmux-bashwrap hook` exits nonzero or with malformed JSON, Claude Code falls back to running the command unmodified (existing hook semantics). User loses streaming; the command still runs. Log the panic and surface it on the next agent activity-log refresh.

### 10.6 50KB head/tail truncation drops critical mid-output errors

The model-visible blob is truncated; the frontend has the full thing. If the model needs the elided content, it can request a re-read. Trade-off accepted.

### 10.7 Backpressure overflow

WPS publish queue at the wrapper saturates → wrapper's reader blocks → kernel PTY buffer fills → user's command writes block. Correct behavior — preferable to dropping chunks silently. Surface a system-kind chunk if back-pressure exceeds 1s.

### 10.8 Audit-ring storm

PR #800's reducer events go to the audit ring. At 5000 lines/sec each chunk produces a `tool-chunk-appended` entry → 5000 ring entries/sec → the ring evicts everything else within milliseconds. PR γ handles this with sampling.

### 10.9 Codex / Gemini providers ship later

PR β is Claude-only. Codex and Gemini users still see the original whole-result behavior until PR δ. Acceptable for staged rollout; document the per-provider matrix in the agent pane.

### 10.10 Auth-key inheritance gap

If a sandboxed Bash environment strips `AGENTMUX_*` env (firejail, container, etc.), wrapper can't authenticate. Graceful degradation per §7 — fall back to buffered-only mode. Surface a system chunk warning.

---

## 11. Alternatives considered (and rejected)

### 11.1 MCP + PreToolUse-deny-redirect

Register `mcp__agentmux__bash` as a streaming MCP tool; `PreToolUse` denies native `Bash` with reason "use mcp__agentmux__bash". Claude is *expected* to retry via the MCP tool.

**Why rejected:**
- Behavior on programmatic deny isn't documented by Anthropic. Could be "retry with named tool" (best), "ask user how to proceed" (middling), "give up" (bad), or "loop" (worst).
- Adds at least one model round-trip per session of friction even in the best case.
- Adds a "did Claude pick the MCP tool?" matrix per provider release. Higher drift risk than the rewrite path.
- The MCP server registration adds complexity for a path we don't end up using.

### 11.2 PATH shim binary

Set `PATH=<shim_dir>:$PATH` in Claude's environment; shim binary named `bash` execs real bash through a tee/PTY-allocating wrapper.

**Why rejected:**
- Requires Claude SDK to do PATH-based shell lookup rather than hardcoding `/bin/bash`. Empirically uncertain and version-fragile.
- Cross-platform shim is heavy (.cmd / .exe / .bat on Win32 vs shell script on Unix).
- Same ANSI/PTY problems unless the shim allocates a PTY itself — at which point we've done the work of option B anyway, without the hook's deterministic guarantee that we're in the loop.

### 11.3 `tee` via command-rewrite

Hook rewrites command to `bash -c 'orig 2>&1 | tee /tmp/foo.log; exit ${PIPESTATUS[0]}'`. AgentMux watches the log file.

**Why rejected:**
- Pipe strips terminal capability — colors/spinners die.
- `tee` interferes with interactive prompts.
- `PIPESTATUS` is bash-only; sh/zsh/Win32 each need their own variant.
- Disk I/O in hot path is 10-50ms slower per batch than memory pipes.

### 11.4 Post-hoc synthesis from `tool_result`

When the whole result lands, split it by line and dispatch fake chunks at 50ms intervals to simulate streaming.

**Why rejected:**
- Indistinguishable from "fake" — the command really did just wait 3 minutes silently and now the UI is pretending it streamed. Strictly worse than no streaming.

### 11.5 Replace `claude` CLI with embedded Agent SDK or direct Anthropic API

Strongest perf + UX path. **Out of scope for this spec.** Requires losing OAuth-via-Claude-Code-subscription as a deployment model unless Anthropic offers third-party OAuth client registration. Tracked in [docs/analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md](../analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md) as a long-horizon discussion.

---

## 12. Files touched (estimate)

| Path | Change | LOC |
|---|---|---|
| `agentmux-bashwrap/src/bash_wrap.rs` (new) | PTY runner, WPS publish, b64 decode, format-for-model | ~280 |
| `agentmux-bashwrap/src/hook.rs` (new) | `hook` subcommand | ~60 |
| `agentmux-bashwrap/src/wps_client.rs` (new) | HTTP client with X-AuthKey injection | ~50 |
| `agentmux-bashwrap/src/main.rs` | Subcommand dispatch | ~30 |
| `agentmux-bashwrap/Cargo.toml` | Add `portable_pty`, `base64`, `reqwest` | ~5 |
| `agentmux-srv/src/backend/agent_config.rs` | Auto-inject hook entry; merge logic | ~50 |
| `agentmux-common/src/wps.rs` | Subject naming + auth helper | ~30 |
| `frontend/app/view/agent/useAgentStream.ts` | Chunk subscription lifecycle | ~80 |
| `frontend/app/store/agent-document/reducer.ts` + tests | Multi-chunk `ToolChunkAppend` | ~60 |
| `frontend/app/store/agent-document/types.ts` | `chunks: ToolLogChunk[]` shape | ~5 |
| `docs/specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md` | Phase 2 section rewritten to point at this spec | — |
| Tests across all of the above | | ~350 |
| **Total** | | **~1000** |

---

## 13. Effort

| Component | Days |
|---|---|
| PR α (frontend overlay UI) | 1.5 |
| PR β (wrapper subcommand + hook + bridge + auth threading) | 3.5 |
| PR γ (RAF coalescing + perf) | 1.0 |
| PR δ (Codex parity) | 1.5 |
| PR δ' (Gemini parity, if hook surface exists) | 1.5 |
| Manual smoke (Win + Linux + mac) | 1.0 |
| **Total (Claude-only)** | **~7 days** |
| **Total (all providers)** | **~10 days** |

---

## 14. Open questions

- **Stdout/stderr separation inside the PTY.** Unix PTYs merge by default; we'd need dup'd file descriptors or a pty-pair-per-stream to distinguish. Worth it for v1, or accept merged-stdout for now and add stderr discrimination as a follow-up? Recommend: v1 ships merged; the wrapper detects writes-from-stderr via process-tracing on Linux as a follow-up.
- **WPS subject naming format.** `tool_chunk:<id>` works. If `<id>` contains characters illegal in WPS subjects, escape or hash. Decide at PR β time.
- **Stdin reverse channel for interactive prompts.** Out of scope for PR β; deferred to PR γ+. The "read -p" scenario fails gracefully (times out) until then. The protocol shape (`stdin:<id>` subject?) gets designed when we actually wire it.
- **`agentmux-bashwrap` discovery on PATH for arbitrary user shells.** We control Claude's env, so PATH is whatever we set. But if a user runs `bash` themselves and that shell tries to invoke our `agentmux-bashwrap`, it won't find it unless we've added the install dir to user PATH globally. Out of scope.
- **Telemetry / observability.** Per-tool latency, chunk rate, PTY allocation success rate. Tag and surface in the existing diag panel.
- **Permissions interaction.** Current production state is `--dangerously-skip-permissions`, so the rewrite never hits the permission UI. If decisions get re-enabled per [SPEC_DECISION_PROMPT_2026_04_24.md](./SPEC_DECISION_PROMPT_2026_04_24.md), we need to decide whether the rewritten command auto-allows (it's still ultimately bash, after all) or shows the *rewritten* command in the prompt (confusing — user sees `agentmux-bashwrap exec --b64...`) or the *original* (need to thread it through). Recommend: prompt shows the original command from the un-rewritten hook input.

---

## 15. Cross-references

- Live-log frontend reducer + UI: [SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md](./SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md)
- Phase 1 implementation (data shape + reducer): [PR #800](https://github.com/agentmuxai/agentmux/pull/800)
- Option analysis: [docs/analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md](../analysis/TOOL_OUTPUT_STREAMING_2026_05_11.md)
- Auth middleware that the wrapper threads through: [PR #801](https://github.com/agentmuxai/agentmux/pull/801)
- Permission gating (interacts if/when re-enabled): [SPEC_DECISION_PROMPT_2026_04_24.md](./SPEC_DECISION_PROMPT_2026_04_24.md)
- MCP server injection: `agentmux-srv/src/backend/agent_config.rs:230-293`
- Hooks writer: `agentmux-srv/src/backend/agent_config.rs:112-119`
- Claude CLI spawn args: `agentmux-srv/src/backend/providers.rs:100-107`
- Subprocess output → WPS pattern (mirror this): `agentmux-srv/src/backend/blockcontroller/subprocess.rs:442-561`
- ToolBlock rendering (replaced in PR α): `frontend/app/view/agent/components/ToolBlock.tsx`
- DocumentRow tool node wiring: `frontend/app/view/agent/virtualization/DocumentRow.tsx:232-238`
- Reducer-stack master status (this lands as a Slice item): [reference_master_reducer_status.md](../../.claude/projects/C--Systems/memory/reference_master_reducer_status.md)
