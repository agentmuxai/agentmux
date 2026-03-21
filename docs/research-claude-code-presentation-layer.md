# Research: Claude Code Presentation Layer — Embedding `claude -p` in a GUI/TUI

**Date:** 2026-03-21
**Scope:** How open-source projects invoke `claude -p --output-format stream-json`, handle credentials, manage sessions, and parse events. Best practices and known pitfalls.

---

## 1. Projects Found

### 1.1 Crystal / Nimbalyst (stravu/crystal)
An Electron desktop app that ran multiple Claude Code sessions in parallel git worktrees. Deprecated February 2026 and superseded by [Nimbalyst](https://nimbalyst.com). The source repo is at https://github.com/stravu/crystal — commits before Feb 2026 show the invocation patterns, but the active implementation has moved to a closed product. Architecture: each worktree gets its own subprocess, session state tracked by session ID.

### 1.2 opcode (winfunc/opcode)
Tauri 2 desktop app (AGPL) for managing Claude Code sessions with custom agents and background execution. Relevant because it's Tauri-based (same stack as AgentMux). Source: https://github.com/winfunc/opcode. Backend is Rust; subprocess invocation is in `src-tauri/src/`. The README does not expose the invocation details but the architecture is identical to what AgentMux does.

### 1.3 claude-flow / ruflo (ruvnet/ruflo)
Agent orchestration framework with "stream-json chaining" — piping `--output-format stream-json` from one Claude instance to `--input-format stream-json` of another. Source: https://github.com/ruvnet/ruflo. Documented pattern for multi-agent pipelines.

### 1.4 format-claude-stream (Khan/format-claude-stream)
CLI filter that converts `stream-json` output to human-readable text. Minimal but shows the bare parsing loop: read lines, check `type` field, accumulate `text_delta` events. Source: https://github.com/Khan/format-claude-stream.

### 1.5 Agent SDK (official)
Anthropic now ships an official Agent SDK (Python: `claude_agent_sdk`, TypeScript: `@anthropic-ai/claude-agent-sdk`). The CLI `-p` mode is explicitly documented as "the Agent SDK via CLI." The SDK wraps the same subprocess internally. For programmatic use in a GUI, the SDK packages are preferable to shelling out, but many GUI apps still shell out for process-level isolation and because Rust/Go FFI to a Node.js SDK is awkward.

---

## 2. Correct Invocation of `claude -p`

### Canonical flags for stream-json embedding

```bash
claude -p \
  --verbose \
  --output-format stream-json \
  --include-partial-messages \
  --dangerously-skip-permissions \
  "Your prompt here"
```

Key points:
- `--verbose` is **required** for stream-json to emit full turn-by-turn output. Without it, only the final message is output (exit code 1 with no content is a common symptom of omitting this).
- `--include-partial-messages` is required to get `stream_event` token deltas. Without it you only get complete `AssistantMessage` objects after each turn completes.
- `-p` / `--print` is the flag that enables non-interactive mode. The old term was "headless mode."
- `--dangerously-skip-permissions` is needed in automated contexts where no human can answer permission prompts.

### Passing the prompt via stdin (pipe)

You can pipe content as the prompt instead of passing it as an argument:

```bash
echo "Summarize this" | claude -p --output-format stream-json --verbose
cat file.txt | claude -p --append-system-prompt "You are a reviewer" --output-format stream-json --verbose
```

For multi-turn over a long-running process using `--input-format stream-json`, write NDJSON lines to stdin. Each line must be a valid JSON message. This mode is documented but has known fragility (see Section 6).

### Argument vs stdin for the prompt

- Argument (`claude -p "prompt"`) — preferred for single-turn, simple prompts.
- Stdin pipe (`echo "prompt" | claude -p`) — required when the prompt contains characters that would be mis-parsed by the shell, or when the content is large.
- `--input-format stream-json` on stdin — for multi-turn long-running sessions (advanced, fragile on Windows).

### Restricting tools

```bash
claude -p --allowedTools "Read,Edit,Bash(git *)" --output-format stream-json --verbose "..."
```

Use `--tools "Bash,Read,Edit"` to restrict which tools are available at all. Use `--allowedTools` to pre-approve specific tools without prompting.

---

## 3. Credential Handling and `CLAUDE_CONFIG_DIR`

### What `CLAUDE_CONFIG_DIR` does

`CLAUDE_CONFIG_DIR` overrides the directory where Claude Code stores all configuration and data:
- `<dir>/settings.json` — user-level settings
- `<dir>/../.claude.json` (i.e., the sibling `~/.claude.json`) — global state including OAuth token
- Session `.jsonl` files under `<dir>/projects/<encoded-cwd>/`

Setting `CLAUDE_CONFIG_DIR=/path/to/agent-A-config` before launching `claude -p` makes that instance use a completely separate config tree.

### Sharing vs. isolating credentials

**Option A: Shared credentials (simplest)**
All agent instances use `~/.claude/` (default). No `CLAUDE_CONFIG_DIR` override needed. All agents share the same OAuth token. This is fine for single-user scenarios where all agents run under the same account.

**Option B: Per-agent CLAUDE_CONFIG_DIR with copied credentials**
For each agent, create a dedicated config directory and copy in the auth files:

```bash
# Create per-agent config dir
mkdir -p ~/.agentmux/agents/AgentA/claude-config

# Copy the credentials files from the default location
cp ~/.claude.json ~/.agentmux/agents/AgentA/claude-config/../claude-agent-a.json
# Note: .claude.json must live at $HOME/.claude.json — CLAUDE_CONFIG_DIR does NOT redirect it
```

**Critical finding:** `CLAUDE_CONFIG_DIR` only redirects the `~/.claude/` directory (settings, sessions, etc.), NOT `~/.claude.json`. The `~/.claude.json` file (which holds the OAuth account UUID and `hasCompletedOnboarding`) always lives at `$HOME/.claude.json` regardless of `CLAUDE_CONFIG_DIR`. This means OAuth token sharing happens through `~/.claude.json` and the system keychain, not through `CLAUDE_CONFIG_DIR`.

**Option C: API key (no OAuth)**
Set `ANTHROPIC_API_KEY` in the environment. In `-p` mode the API key is always used when present, bypassing OAuth entirely. This is the cleanest isolation strategy for multi-agent deployments:

```bash
ANTHROPIC_API_KEY="sk-ant-..." CLAUDE_CONFIG_DIR="/path/to/agent-A" claude -p ...
```

Each agent can have its own API key (if provisioned separately) or share one key but have isolated config dirs for session storage.

### The macOS Keychain credential collision bug (issue #20553)

On macOS, Claude Code stores OAuth refresh tokens in the Keychain under the service name `Claude Code-credentials` — a **single, shared entry** regardless of `CLAUDE_CONFIG_DIR`. This means:

1. Profile A logs in → tokens stored in `Claude Code-credentials`
2. Profile B logs in → **overwrites** the same entry
3. Profile A's refresh token is now invalid → 401 errors after ~8 hours

**Status:** Open as of March 2026, version 2.1.19+. Affects macOS only (Windows uses a different credential store; Linux has no system keychain by default).

**Workaround:** Use `ANTHROPIC_API_KEY` for one or more profiles to avoid the keychain collision. Do not mix OAuth and API key auth in the same environment — if `ANTHROPIC_API_KEY` is set, it takes precedence over OAuth in `-p` mode.

### Automated / headless auth (no browser)

For CI/CD and embedded contexts where browser-based OAuth is impossible:

1. On a machine with a browser, run `claude setup-token` to generate a long-lived OAuth token (valid ~1 year).
2. Set `CLAUDE_CODE_OAUTH_TOKEN=<token>` in the environment.
3. Create `~/.claude.json` with `hasCompletedOnboarding: true` to skip the onboarding flow:

```json
{
  "hasCompletedOnboarding": true,
  "lastOnboardingVersion": "2.1.29",
  "oauthAccount": {
    "accountUuid": "your-account-uuid",
    "emailAddress": "your@email.com",
    "organizationUuid": "your-org-uuid"
  }
}
```

Set permissions on this file to `0600`.

**Do not set both `CLAUDE_CODE_OAUTH_TOKEN` and `ANTHROPIC_API_KEY` simultaneously** — they conflict.

### Authentication precedence in `-p` mode

1. `ANTHROPIC_API_KEY` (always wins in `-p` mode if set)
2. `CLAUDE_CODE_OAUTH_TOKEN` (long-lived token set in env)
3. OAuth tokens from keychain / `~/.claude/` (interactive login artifacts)

In interactive mode, the user is prompted to confirm if `ANTHROPIC_API_KEY` is present. In `-p` mode it is used silently.

---

## 4. Session Continuity with `--resume`

### Session storage location

Sessions are stored as NDJSON files:
```
~/.claude/projects/<encoded-cwd>/<session-id>.jsonl
```

Where `<encoded-cwd>` is the absolute working directory with every non-alphanumeric character replaced by `-` (e.g., `/Users/me/proj` → `-Users-me-proj`).

When `CLAUDE_CONFIG_DIR` is set, the projects directory moves to `<CLAUDE_CONFIG_DIR>/projects/`.

### Extracting the session ID

From `--output-format json` (non-streaming), the session ID is in the top-level `session_id` field:

```bash
session_id=$(claude -p "Start a review" --output-format json | jq -r '.session_id')
```

From `--output-format stream-json`, the session ID appears in two places:
- The first event: `{"type":"system","subtype":"init","session_id":"..."}`
- The last event: `{"type":"result","session_id":"..."}`

Extract from the init event:
```bash
session_id=$(claude -p ... --output-format stream-json --verbose | \
  jq -r 'select(.type=="system" and .subtype=="init") | .session_id' | head -1)
```

### Using `--resume`

Resume a specific session by ID or by name:

```bash
# Resume by UUID
claude -p "Continue the review" --resume "$session_id" --output-format stream-json --verbose

# Resume by name (set with --name on first call)
claude -p "Continue" --resume "my-review" --output-format stream-json --verbose
```

Resume by most-recent session in the current directory:

```bash
claude -p "Continue" --continue --output-format stream-json --verbose
```

### Fork to branch without losing history

```bash
claude -p "Try approach B" --resume "$session_id" --fork-session --output-format json
```

The forked session gets a new UUID; the original is unchanged.

### Cross-host resume

Sessions are local to the machine. To resume on a different host (e.g., in a container), copy the `.jsonl` file to the same path on the new host. The `cwd` encoding must match exactly — if the session was created in `/workspace/project`, the resume call must also run from `/workspace/project`.

**Common failure mode:** if `--resume` produces a fresh session instead of restoring history, the working directory doesn't match the encoded path in the filename.

### Token budget for multi-turn sessions

Multi-turn sessions accumulate up to 200,000 tokens of context. In practice, multi-turn runs consume 30–50% more tokens per turn than fresh sessions, but avoid the re-analysis cost of starting over. For 3-turn workflows, multi-turn uses ~40% fewer total tokens than three isolated calls.

---

## 5. Parsing `stream-json` Events

### Top-level message types

Every line from `claude -p --output-format stream-json --verbose` is a JSON object with a `type` field:

| `type` | `subtype` | Description |
|--------|-----------|-------------|
| `system` | `init` | First event. Contains `session_id`, `tools[]`, `model`, `version`. |
| `system` | `api_retry` | Emitted before a retry. Contains `attempt`, `max_retries`, `retry_delay_ms`, `error_status`, `error`. |
| `assistant` | — | Complete assistant turn (when `--include-partial-messages` is off). Contains `message` with full content blocks. |
| `user` | — | Tool results injected back into the conversation. Contains `message` with `tool_result` blocks. |
| `stream_event` | — | Raw API streaming event (only with `--include-partial-messages`). Contains `event` object. |
| `result` | `success` | Final event. Contains `session_id`, `cost_usd`, `num_turns`, `result` (text). |
| `result` | `error_max_turns` | Stopped due to `--max-turns` limit. |
| `result` | `error_max_budget_usd` | Stopped due to `--max-budget-usd` limit. |
| `result` | `error_during_execution` | Unhandled error during the agent loop. |

### `stream_event` subtypes (the `event.type` field)

These are raw Claude API streaming events:

| `event.type` | Description |
|---|---|
| `message_start` | Start of a new message. Contains usage metadata. |
| `content_block_start` | New content block. `content_block.type` is `text` or `tool_use`. |
| `content_block_delta` | Incremental content. `delta.type` is `text_delta` (text chunk) or `input_json_delta` (tool input chunk). |
| `content_block_stop` | End of content block. |
| `message_delta` | Message-level update: stop reason, cumulative usage. |
| `message_stop` | End of the message. |

### Minimal streaming text extractor (jq)

```bash
claude -p "Write a poem" \
  --output-format stream-json --verbose --include-partial-messages | \
  jq -rj 'select(.type=="stream_event" and .event.delta.type?=="text_delta") | .event.delta.text'
```

### Minimal streaming parser (Rust pseudocode)

```rust
for line in child_stdout.lines() {
    let event: Value = serde_json::from_str(&line)?;
    match event["type"].as_str() {
        Some("system") if event["subtype"] == "init" => {
            session_id = event["session_id"].as_str().map(String::from);
        }
        Some("stream_event") => {
            let inner = &event["event"];
            if inner["type"] == "content_block_delta" {
                if inner["delta"]["type"] == "text_delta" {
                    ui_stream_text(inner["delta"]["text"].as_str().unwrap_or(""));
                }
            }
        }
        Some("assistant") => {
            // Full message when --include-partial-messages is off
            // event["message"]["content"] is an array of content blocks
        }
        Some("result") => {
            let cost = event["cost_usd"].as_f64();
            let subtype = event["subtype"].as_str();
            handle_result(subtype, cost);
            // After this event, the process should exit (but see Section 6)
        }
        _ => {}
    }
}
```

### Tool call streaming

Tool calls arrive as a sequence:
1. `content_block_start` with `content_block.type == "tool_use"` and `content_block.name`
2. Multiple `content_block_delta` with `delta.type == "input_json_delta"` containing partial JSON
3. `content_block_stop` — the full input JSON is now accumulated
4. A `user` message event arrives with the `tool_result` after execution

### Cost extraction

The `result` event contains `cost_usd` (not `total_cost` — a common mistake). This is the cost for the current session turn, not cumulative.

---

## 6. Known Gotchas and Exit Code 1 Causes

### 6.1 Process hangs and never exits (most common)

**Issue #25629, #21099:** After successfully emitting the `result` event, `claude -p` may hang indefinitely with stdout still open. The process remains alive requiring manual `SIGKILL`.

- Affects: Linux (confirmed), possibly all platforms.
- Frequency: Intermittent; more likely on longer sessions (80+ turns).
- Root cause: Pending Node.js timers or MCP server connections prevent clean exit.

**Workaround:**
```rust
// After receiving the result event, start a kill timer
if event_type == "result" {
    tokio::time::sleep(Duration::from_secs(30)).await;
    child.kill().await.ok();
}
```
Or use `CLAUDE_CODE_EXIT_AFTER_STOP_DELAY` env var (in milliseconds) to trigger automatic exit after the query loop goes idle.

### 6.2 Missing final `result` event (intermittent)

**Issue #1920:** The process completes work but never emits the `{"type":"result"}` event. The process eventually exits with code 0 but the consumer hangs waiting for the result.

**Workaround:** Implement a timeout after the last `assistant` message event. If no `result` arrives within N seconds, kill the process and treat the last assistant message content as the result.

### 6.3 Exit code 1 with no output: missing `--verbose`

Running `claude -p "..." --output-format stream-json` without `--verbose` outputs only the final message text, not the stream events. In many cases this produces exit code 1 with an error on stderr or simply empty stdout.

**Fix:** Always include `--verbose` with `stream-json`.

### 6.4 Exit code 1: authentication failure

Most common causes:
- `ANTHROPIC_API_KEY` is set in the environment but is invalid or from a disabled organization.
- OAuth token has expired (`~/.claude.json` has stale credentials).
- `CLAUDE_CONFIG_DIR` points to an empty/new directory with no credentials.
- Both `CLAUDE_CODE_OAUTH_TOKEN` and `ANTHROPIC_API_KEY` are set simultaneously.

**Diagnosis:**
```bash
claude auth status --json
# Returns: { "loggedIn": bool, "authMethod": "...", "email": "..." }
# Exits 0 if logged in, 1 if not.
```

**In non-interactive mode**, `ANTHROPIC_API_KEY` is always used if set, even if the user is OAuth-logged-in. If a stale or wrong API key is in the environment, it silently overrides OAuth.

### 6.5 Exit code 1: interactive prompts blocking

Claude Code may emit interactive prompts (onboarding, theme selection, login) to stdout/stderr that block execution. These only appear when `-p` is not set. With `-p`, they should be suppressed.

If they appear anyway (e.g., first run with a new `CLAUDE_CONFIG_DIR`), pre-create `~/.claude.json` (or the equivalent under the config dir) with `hasCompletedOnboarding: true`.

### 6.6 Exit code 1: permission prompt in `-p` mode

If a tool requires user permission and `--dangerously-skip-permissions` is not set, the process will block waiting for stdin input it will never receive. The result is a hang, not an immediate exit. Always set `--dangerously-skip-permissions` or pre-configure `--allowedTools` to cover all expected tool uses.

### 6.7 `--input-format stream-json` stdin pipe hang (issue #3187)

Multi-turn via stdin NDJSON worked intermittently, especially on Windows via WSL. The first turn succeeds; subsequent `writeLine()` + `flush()` calls hang. This appears to be a buffering/fd-inheritance problem in certain shell environments.

**Workaround:** Use `--resume <session-id>` with separate process invocations instead of a long-lived stdin pipe for multi-turn. This is more robust and avoids the buffering issues entirely.

### 6.8 Windows-specific: Git Bash stdout fd inheritance

On Windows with Git Bash (MSYS2), Node.js passes a Windows HANDLE as fd 1 to the child process. MSYS2 cannot convert it to a valid POSIX fd. This causes the subprocess to exit with `Bad file descriptor` on first write to stdout.

This is the same issue documented in the AgentMux MEMORY.md. The fix used in AgentMux is to pass a BASH_ENV workaround that reopens fd 1 from the symlink target.

For spawning `claude -p` from a Tauri/Rust backend, use Rust's `Command` API directly (not bash subprocess wrapping). Rust's `std::process::Command` handles Windows handle inheritance correctly. Pass the prompt as an argument, not via bash piping.

### 6.9 Auto-updater interfering with headless operation

If `DISABLE_AUTOUPDATER` is not set, Claude Code may attempt to auto-update mid-session. In long-running embedded contexts, set:

```bash
DISABLE_AUTOUPDATER=1
DISABLE_TELEMETRY=1
CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1  # equivalent of all DISABLE_* flags combined
```

---

## 7. Recommended Architecture for AgentMux

Based on all findings, the recommended architecture for embedding `claude -p` in AgentMux:

### 7.1 Process invocation (Rust)

```rust
let mut cmd = Command::new("claude");
cmd.args([
    "-p",
    "--verbose",
    "--output-format", "stream-json",
    "--include-partial-messages",
    "--dangerously-skip-permissions",
    "--allowedTools", "Bash,Read,Edit,Write,Glob,Grep",
]);

// Session continuity
if let Some(session_id) = &resume_session_id {
    cmd.args(["--resume", session_id]);
}

// Agent isolation
cmd.env("CLAUDE_CONFIG_DIR", agent_config_dir);
cmd.env("DISABLE_AUTOUPDATER", "1");
cmd.env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");

// Credential: prefer API key for isolation; fall back to shared OAuth
if let Some(api_key) = agent_api_key {
    cmd.env("ANTHROPIC_API_KEY", api_key);
} else {
    // Shared OAuth — all agents use the same ~/.claude.json
    // Do NOT set CLAUDE_CONFIG_DIR to an empty dir without credentials
}

// Pass prompt as argument (avoids stdin buffering issues on Windows)
cmd.arg(&prompt_text);

cmd.stdout(Stdio::piped());
cmd.stderr(Stdio::piped());
```

### 7.2 Session ID capture

Parse the first `system/init` event to capture the session ID. Store it for use with `--resume` on the next message.

### 7.3 Exit handling

After receiving the `result` event:
1. Start a 30-second kill timer.
2. If stdout closes before the timer fires, cancel the timer.
3. If the timer fires, `SIGKILL` the process.

Do not rely on the process exiting cleanly after the `result` event.

### 7.4 Credential isolation strategy

For AgentMux's multi-agent scenario:

- **Simplest (single user):** Share `~/.claude/` (default). All agents use the same OAuth. `CLAUDE_CONFIG_DIR` set per agent only for session file isolation.
- **Multi-user or compliance:** Use `ANTHROPIC_API_KEY` per agent. Set `CLAUDE_CONFIG_DIR` per agent for full isolation. Pre-populate each agent's config dir with a minimal `settings.json` (so no interactive prompts fire on first run).
- **Avoid on macOS:** Multiple OAuth logins with different `CLAUDE_CONFIG_DIR` values collide in the Keychain (bug #20553, open as of March 2026). Use API key auth instead.

### 7.5 Multi-turn conversation pattern

```
Turn 1:
  spawn: claude -p --verbose --output-format stream-json --include-partial-messages "<prompt>"
  capture: session_id from system/init event
  stream: text_delta events to UI
  wait: for result event
  kill: after result + 30s timeout

Turn N:
  spawn: claude -p --verbose --output-format stream-json --include-partial-messages \
         --resume "<session_id>" "<next_prompt>"
  (same streaming/kill pattern)
```

This is more robust than the long-lived stdin pipe approach, avoids the `--input-format stream-json` buffering bugs, and works on Windows.

### 7.6 Avoid

- Do not use `--input-format stream-json` with a long-lived process on Windows — use separate invocations with `--resume` instead.
- Do not omit `--verbose` — stream-json is silent without it.
- Do not set `ANTHROPIC_API_KEY` to an empty string — this is treated as "key present but invalid" in `-p` mode.
- Do not leave `ANTHROPIC_API_KEY` in the environment if you want OAuth to take effect.
- Do not assume the process exits after the `result` event — always have a kill timeout.

---

## 8. Summary of Key Environment Variables

| Variable | Purpose | AgentMux Use |
|---|---|---|
| `CLAUDE_CONFIG_DIR` | Redirect config/session storage | Set per agent for session isolation |
| `ANTHROPIC_API_KEY` | API key; overrides OAuth in `-p` mode | Set per agent if multi-account needed |
| `CLAUDE_CODE_OAUTH_TOKEN` | Long-lived OAuth token (no browser) | Use for CI/headless first-time setup |
| `DISABLE_AUTOUPDATER` | Prevent mid-session updates | Always set to `1` in embedded contexts |
| `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` | Disable telemetry, error reporting, autoupdate | Set to `1` in production |
| `CLAUDE_CODE_EXIT_AFTER_STOP_DELAY` | Auto-kill N ms after idle | Set to `30000` as hang mitigation |
| `CLAUDE_CODE_SHELL` | Override shell detection | Set if default shell detection fails |

---

## Sources

- [Run Claude Code programmatically (official docs)](https://code.claude.com/docs/en/headless)
- [CLI reference (official docs)](https://code.claude.com/docs/en/cli-reference)
- [Environment variables (official docs)](https://code.claude.com/docs/en/env-vars)
- [Work with sessions — Agent SDK docs](https://platform.claude.com/docs/en/agent-sdk/sessions)
- [Stream responses in real-time — Agent SDK docs](https://platform.claude.com/docs/en/agent-sdk/streaming-output)
- [Secure deployment guide](https://platform.claude.com/docs/en/agent-sdk/secure-deployment)
- [Troubleshooting (official docs)](https://code.claude.com/docs/en/troubleshooting)
- [Issue #20553: OAuth credential isolation failure with CLAUDE_CONFIG_DIR (macOS Keychain collision)](https://github.com/anthropics/claude-code/issues/20553)
- [Issue #25629: claude -p hangs after result event](https://github.com/anthropics/claude-code/issues/25629)
- [Issue #1920: Missing final result event in stream-json mode](https://github.com/anthropics/claude-code/issues/1920)
- [Issue #3187: input-format stream-json stdin hang](https://github.com/anthropics/claude-code/issues/3187)
- [Issue #24596: DOCS — stream-json event type reference missing](https://github.com/anthropics/claude-code/issues/24596)
- [Headless auth gist (coenjacobs)](https://gist.github.com/coenjacobs/d37adc34149d8c30034cd1f20a89cce9)
- [Crystal/Nimbalyst (stravu/crystal)](https://github.com/stravu/crystal)
- [opcode (winfunc/opcode)](https://github.com/winfunc/opcode)
- [format-claude-stream (Khan)](https://github.com/Khan/format-claude-stream)
- [awesome-claude-code (hesreallyhim)](https://github.com/hesreallyhim/awesome-claude-code)
