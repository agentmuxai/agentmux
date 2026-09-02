# Subprocess Transport — Implementation Spec

> Migrate the agent execution path from PTY-based shell injection to direct subprocess transport using the Claude Agent SDK's stdin/stdout JSON-lines protocol.

**Status:** Implementation plan
**Depends on:** `specs/presentation-layer.md` (architecture)
**Branch:** `agentx/subprocess-transport`

---

## Scope

Replace the current PTY + bootstrap-script approach with a `SubprocessController` that:
1. Spawns the CLI as a child process with piped stdin/stdout
2. Reads NDJSON from stdout, persists to `.jsonl`, publishes via WPS
3. Accepts JSON user messages from the frontend and writes to stdin
4. Handles process lifecycle (start, stop, restart, crash recovery)

The PTY path (`ShellController` + `bootstrap.ts`) stays as a legacy fallback for non-SDK providers.

---

## Phase 1: Backend — SubprocessController

### 1.1 New file: `blockcontroller/subprocess.rs`

A new controller type alongside the existing `ShellController`. Manages a single child process per block.

```rust
pub struct SubprocessController {
    block_id: String,
    child: Option<tokio::process::Child>,
    stdin_tx: Option<tokio::sync::mpsc::Sender<String>>,  // channel to write to stdin
    output_file: PathBuf,                                   // .jsonl persistence path
    session_id: Option<String>,                             // captured from init message
    state: SubprocessState,                                 // spawning/running/exited/failed
}

pub enum SubprocessState {
    Idle,
    Spawning,
    Running { pid: u32 },
    Exited { code: i32 },
    Failed { error: String },
}
```

**Spawn flow:**
```rust
impl SubprocessController {
    pub async fn spawn(&mut self, config: SpawnConfig) -> Result<(), ControllerError> {
        // 1. Build Command
        let mut cmd = tokio::process::Command::new(&config.cli_command);
        cmd.args(&config.cli_args);
        cmd.current_dir(&config.working_dir);
        cmd.envs(&config.env_vars);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // 2. Spawn
        let mut child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);
        self.state = SubprocessState::Running { pid };

        // 3. Take ownership of stdin/stdout
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // 4. Start stdin writer task
        let (stdin_tx, stdin_rx) = tokio::sync::mpsc::channel::<String>(64);
        self.stdin_tx = Some(stdin_tx);
        tokio::spawn(stdin_writer(stdin, stdin_rx));

        // 5. Start stdout reader task
        let block_id = self.block_id.clone();
        let output_file = self.output_file.clone();
        tokio::spawn(stdout_reader(stdout, block_id, output_file, wstore, broker));

        // 6. Start stderr logger task
        tokio::spawn(stderr_logger(stderr, self.block_id.clone()));

        // 7. Start process wait task
        self.child = Some(child);
        tokio::spawn(process_waiter(child, self.block_id.clone(), broker));

        Ok(())
    }
}
```

**SpawnConfig:**
```rust
pub struct SpawnConfig {
    pub cli_command: String,          // "claude" (resolved from PATH or explicit)
    pub cli_args: Vec<String>,        // ["-p", "--input-format", "stream-json", ...]
    pub working_dir: PathBuf,         // agent working directory
    pub env_vars: HashMap<String, String>,  // ANTHROPIC_API_KEY, etc.
    pub output_file: PathBuf,         // where to persist .jsonl
}
```

### 1.2 stdout_reader task

```rust
async fn stdout_reader(
    stdout: tokio::process::ChildStdout,
    block_id: String,
    output_file: PathBuf,
    wstore: Arc<WaveStore>,
    broker: Arc<Broker>,
) {
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_file)
        .await
        .expect("failed to open output file");

    while let Ok(Some(line)) = lines.next_line().await {
        // 1. Persist to .jsonl
        file.write_all(line.as_bytes()).await.ok();
        file.write_all(b"\n").await.ok();

        // 2. Store in FileStore (for reconnection)
        wstore.blockfile_append(&block_id, "output", line.as_bytes()).ok();

        // 3. Publish WPS event to frontend
        let data64 = base64::engine::general_purpose::STANDARD.encode(
            format!("{}\n", line).as_bytes()
        );
        broker.publish(BlockFileEvent {
            block_id: block_id.clone(),
            file_name: "output".to_string(),
            file_op: "append".to_string(),
            data64,
        });
    }

    // stdout closed = process exited (or about to)
    tracing::info!(block_id = %block_id, "subprocess stdout closed");
}
```

### 1.3 stdin_writer task

```rust
async fn stdin_writer(
    mut stdin: tokio::process::ChildStdin,
    mut rx: tokio::sync::mpsc::Receiver<String>,
) {
    while let Some(msg) = rx.recv().await {
        if let Err(e) = stdin.write_all(msg.as_bytes()).await {
            tracing::error!("stdin write error: {e}");
            break;
        }
        if let Err(e) = stdin.write_all(b"\n").await {
            tracing::error!("stdin newline error: {e}");
            break;
        }
        stdin.flush().await.ok();
    }
}
```

### 1.4 process_waiter task

```rust
async fn process_waiter(
    mut child: tokio::process::Child,
    block_id: String,
    broker: Arc<Broker>,
) {
    match child.wait().await {
        Ok(status) => {
            let code = status.code().unwrap_or(-1);
            tracing::info!(block_id = %block_id, exit_code = code, "subprocess exited");
            broker.publish(ProcessExitEvent {
                block_id,
                exit_code: code,
                signal: None,
            });
        }
        Err(e) => {
            tracing::error!(block_id = %block_id, error = %e, "subprocess wait error");
            broker.publish(ProcessExitEvent {
                block_id,
                exit_code: -1,
                signal: None,
            });
        }
    }
}
```

### 1.5 Controller registration

Register `SubprocessController` alongside `ShellController` in the block controller dispatch:

```rust
// blockcontroller/mod.rs
match controller_type {
    "shell" => ShellController::new(block_id, ...),
    "subprocess" => SubprocessController::new(block_id, ...),
    _ => return Err(...)
}
```

---

## Phase 2: Backend — RPC Commands

### 2.1 `SubprocessSpawnCommand`

**Route:** `POST /api/subprocess/spawn`
**RPC name:** `subprocessspawn`

```typescript
// Request
{
    blockid: string;
    cli_command: string;      // "claude"
    cli_args: string[];       // ["-p", "--input-format", "stream-json", ...]
    working_dir: string;
    env_vars: Record<string, string>;
}

// Response (empty on success, error on failure)
```

**Backend handler:**
1. Resolve CLI command (check PATH, check installed version)
2. Prepare `SpawnConfig` from request
3. Create or get `SubprocessController` for the block
4. Call `controller.spawn(config)`
5. Return success/error

### 2.2 `AgentInputCommand`

**Route:** `POST /api/subprocess/input`
**RPC name:** `agentinput`

```typescript
// Request
{
    blockid: string;
    message: string;  // JSON string: {"type":"user","message":{...}}
}
```

**Backend handler:**
1. Get `SubprocessController` for block
2. Send message through `stdin_tx` channel
3. Return success/error

### 2.3 `AgentStopCommand`

**Route:** `POST /api/subprocess/stop`
**RPC name:** `agentstop`

```typescript
// Request
{
    blockid: string;
    force: boolean;  // false = SIGTERM, true = SIGKILL
}
```

**Backend handler:**
1. Get `SubprocessController` for block
2. If `force`: kill process
3. Else: send SIGTERM, wait up to 5s, then kill
4. Cleanup stdin/stdout tasks

---

## Phase 3: Frontend — Launch Path

### 3.1 Update `agent-model.ts`

Add a new `launchForgeAgentSubprocess()` method alongside the existing `launchForgeAgent()`:

```typescript
async launchForgeAgentSubprocess(agent: ForgeAgent): Promise<void> {
    // 1. Load content + skills (same as current)
    const content = await RpcApi.GetAllForgeContentCommand(TabRpcClient, agent.id);
    const skills = await RpcApi.ListForgeSkillsCommand(TabRpcClient, agent.id);

    // 2. Write config files to disk via backend RPC
    const workDir = `~/.agentmux/agents/${agent.id}`;
    await this.writeAgentConfigFiles(workDir, content, skills);

    // 3. Set block metadata
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref: makeORef("block", this.blockId),
        meta: {
            "agent:id": agent.id,
            "agent:name": agent.name,
            "agent:provider": agent.provider,
            "agent:outputformat": "claude-stream-json",
            "controller": "subprocess",
        }
    });

    // 4. Determine CLI args
    const provider = PROVIDERS[agent.provider];
    const cliArgs = [
        "-p",
        "--input-format", "stream-json",
        "--output-format", "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--append-system-prompt-file", `${workDir}/CLAUDE.md`,
        "--mcp-config", `${workDir}/.mcp.json`,
        "--allowedTools", provider.styledArgs.join(","),
    ];

    // 5. Parse env vars from Forge content
    const envVars = this.parseEnvContent(content.env);

    // 6. Spawn subprocess
    await RpcApi.SubprocessSpawnCommand(TabRpcClient, {
        blockid: this.blockId,
        cli_command: provider.cliCommand,
        cli_args: cliArgs,
        working_dir: workDir,
        env_vars: envVars,
    });

    // 7. Send initial prompt (if any)
    // The agent starts waiting for input after init message
}
```

### 3.2 Update `AgentFooter` input handler

```typescript
// Current (PTY mode):
const inputData = stringToBase64(text + "\n");
await RpcApi.ControllerInputCommand(TabRpcClient, {
    blockid,
    inputdata64: inputData,
});

// New (subprocess mode):
const message = JSON.stringify({
    type: "user",
    message: {
        role: "user",
        content: [{ type: "text", text }]
    }
});
await RpcApi.AgentInputCommand(TabRpcClient, {
    blockid,
    message,
});
```

The `AgentFooter` checks block metadata `controller` field to determine which path to use.

### 3.3 Update `useAgentStream.ts`

Minimal change — switch file subject based on controller type:

```typescript
// Current:
const fileSubject = getFileSubject(blockId, "term");

// New:
const controllerType = getBlockMeta(blockId, "controller");
const fileSubject = getFileSubject(
    blockId,
    controllerType === "subprocess" ? "output" : "term"
);
```

The rest of the pipeline (line buffer, JSON.parse, translator, parser) is unchanged.

---

## Phase 4: Config File Writing

### 4.1 New RPC: `WriteAgentConfigCommand`

Instead of heredoc injection into a PTY, write files via a backend RPC:

```typescript
// Request
{
    agent_id: string;
    files: Array<{
        path: string;    // relative to workDir
        content: string;
    }>;
}
```

**Backend handler:**
1. Resolve agent working directory
2. `create_dir_all` if needed
3. Write each file atomically (write to `.tmp`, rename)
4. Return success

Files written:
- `CLAUDE.md` — soul + agentmd + memory + skills index
- `.mcp.json` — MCP server configuration
- `.env` — (optional) env file for reference

### 4.2 Environment variables

Instead of `export KEY=VALUE` in a shell, env vars are passed directly to the subprocess:

```rust
cmd.envs(&config.env_vars);
```

The Forge `env` content is parsed client-side:
```typescript
parseEnvContent(envContent: string): Record<string, string> {
    const vars: Record<string, string> = {};
    for (const line of envContent.split("\n")) {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith("#")) continue;
        const eqIdx = trimmed.indexOf("=");
        if (eqIdx > 0) {
            const key = trimmed.slice(0, eqIdx).trim();
            let val = trimmed.slice(eqIdx + 1).trim();
            // Strip surrounding quotes
            if ((val.startsWith('"') && val.endsWith('"')) ||
                (val.startsWith("'") && val.endsWith("'"))) {
                val = val.slice(1, -1);
            }
            vars[key] = val;
        }
    }
    return vars;
}
```

---

## Phase 5: Session Management

### 5.1 Capture session ID

The first stdout message is a `system/init` message containing `session_id`:

```json
{"type":"system","subtype":"init","session_id":"550e8400-...","tools":[...]}
```

The `stdout_reader` detects this and stores it in block metadata:

```rust
if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
    if parsed.get("type").and_then(|v| v.as_str()) == Some("system")
        && parsed.get("subtype").and_then(|v| v.as_str()) == Some("init")
    {
        if let Some(sid) = parsed.get("session_id").and_then(|v| v.as_str()) {
            wstore.set_block_meta(&block_id, "agent:sessionid", sid).ok();
        }
    }
}
```

### 5.2 Resume session

When the user re-opens a block that has a stored `session_id`:

```typescript
const sessionId = getBlockMeta(blockId, "agent:sessionid");
if (sessionId) {
    cliArgs.push("--resume", sessionId);
}
```

### 5.3 Multi-turn conversations

After the first `ResultMessage` (the CLI completes its agentic loop), the CLI exits in `-p` mode. For ongoing conversation:

**Option A — Re-spawn with `--resume`:**
Each user message spawns a new subprocess with `--resume <session-id>`. The CLI picks up full context from its session store. This is the simplest approach and matches how the Agent SDKs work.

**Option B — Keep alive with `--input-format stream-json`:**
The subprocess stays alive, reading additional user messages from stdin after each result. This requires the CLI to support multi-turn stdin mode (needs verification against Claude Code behavior).

**Recommendation:** Start with Option A. It's simpler, proven, and the session resume is fast since context is already in Claude Code's session store.

---

## Phase 6: Reconnection

### 6.1 On frontend attach (reconnect/refresh)

```typescript
// In useAgentStream.ts, on mount:
const controllerType = getBlockMeta(blockId, "controller");
if (controllerType === "subprocess") {
    // 1. Read full .jsonl content from FileStore
    const existingData = await RpcApi.BlockFileReadCommand(TabRpcClient, {
        blockid: blockId,
        filename: "output",
    });

    // 2. Process all existing lines through translator + parser
    if (existingData) {
        const text = base64ToString(existingData);
        const lines = text.split("\n").filter(Boolean);
        for (const line of lines) {
            processLine(line);  // same pipeline as live events
        }
    }

    // 3. Subscribe to live events (new lines appended after reconnect)
    const fileSubject = getFileSubject(blockId, "output");
    subscribe(fileSubject, onNewEvent);
}
```

### 6.2 On backend restart

If the backend restarts while a subprocess is running:
1. The subprocess is orphaned (no stdin reader, no stdout consumer)
2. On backend startup, check for orphaned .jsonl files
3. Mark corresponding blocks as `exited` / `disconnected`
4. User can click "Restart" to re-spawn with `--resume`

---

## Phase 7: Process Lifecycle UI

### 7.1 Process state events

The `processAtom` signal is updated based on WPS events:

| Event | processAtom state |
|-------|-------------------|
| `SubprocessSpawnCommand` sent | `{ status: "starting" }` |
| First `stream_event` received | `{ status: "running", pid }` |
| `ResultMessage` received | `{ status: "idle" }` (waiting for next prompt) |
| `process:exit` (code 0) | `{ status: "exited", code: 0 }` |
| `process:exit` (code != 0) | `{ status: "crashed", code }` |
| `AgentStopCommand` sent | `{ status: "stopping" }` |

### 7.2 Header status indicator

```
[icon] Agent Name          [running ●]  [x]
[icon] Agent Name          [idle ○]     [x]
[icon] Agent Name          [crashed ✗]  [▶ restart] [x]
```

### 7.3 Restart flow

On crash or manual stop:
1. Read `agent:sessionid` from block metadata
2. Spawn new subprocess with `--resume <session-id>`
3. Frontend replays existing document from FileStore + subscribes to new events

---

## Implementation Order

| Priority | Phase | What | Risk |
|----------|-------|------|------|
| P0 | 1.1-1.4 | SubprocessController core | Medium — new Rust module |
| P0 | 2.1-2.2 | Spawn + Input RPCs | Low — standard HTTP handlers |
| P0 | 3.1-3.3 | Frontend launch + stream switch | Low — parallel to existing path |
| P1 | 4.1-4.2 | Config file writing RPC | Low — file I/O |
| P1 | 5.1-5.3 | Session management | Medium — multi-turn behavior TBD |
| P1 | 6.1-6.2 | Reconnection | Low — FileStore already exists |
| P2 | 7.1-7.3 | Process lifecycle UI | Low — SolidJS signals ready |
| P2 | 2.3 | Stop command | Low |

**P0 delivers:** Working subprocess transport for Claude Code agents. User can click agent, type messages, see rich output. No PTY involved.

**P1 delivers:** Session persistence, resume, reconnection after refresh.

**P2 delivers:** Process status indicators, restart, stop.

---

## Testing

### Manual test plan

1. **Basic flow:** Click Claude agent -> subprocess spawns -> type prompt -> see structured output in document view
2. **Multi-turn:** Send prompt -> wait for result -> send follow-up -> verify context preserved
3. **Reconnection:** Refresh browser mid-session -> verify document replays from .jsonl
4. **Crash recovery:** Kill subprocess externally -> verify UI shows crashed state -> click restart -> verify session resumes
5. **PTY fallback:** Switch agent controller to "shell" -> verify old PTY path still works
6. **Cross-platform:** Test on Windows (pwsh/cmd), macOS, Linux

### Verification commands

```bash
# Verify Claude Code subprocess transport works standalone:
echo '{"type":"user","message":{"role":"user","content":[{"type":"text","text":"What is 2+2?"}]}}' | \
  claude -p --input-format stream-json --output-format stream-json --verbose

# Should produce NDJSON on stdout:
# {"type":"system","subtype":"init",...}
# {"type":"stream_event","event":{"type":"message_start",...},...}
# {"type":"stream_event","event":{"type":"content_block_delta",...},...}
# ...
# {"type":"result","result":"4",...}
```

---

## Migration Notes

- The PTY path (`ShellController`, `bootstrap.ts`, `ControllerInputCommand`) is NOT removed. It stays as a fallback for providers that don't support subprocess transport.
- The `controller` block metadata field (`"shell"` vs `"subprocess"`) determines which path is used.
- Both paths can coexist — different agents can use different controllers.
- The frontend `useAgentStream` already handles both `"term"` and `"output"` subjects based on controller type.
