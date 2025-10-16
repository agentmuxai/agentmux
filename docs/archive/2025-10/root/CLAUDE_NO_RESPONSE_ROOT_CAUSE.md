# Claude Agent No Response - Root Cause Analysis

**Date:** 2025-10-15
**Agent:** AgentX
**Version:** 0.3.11

---

## Problem Statement

When sending input to a spawned Claude agent through AgentMux Desktop, the input is successfully sent to stdin but Claude never responds with any output on stdout/stderr.

## Symptoms

From the debug console logs at 07:44:28:

```
✅ 🔌 WEBSOCKET: [127.0.0.1:54742] ✓ Successfully sent to stdin channel
ℹ️ 📥 [PythonProjects] STDIN: → Sending input #1 (6 bytes)
✅ 📥 [PythonProjects] STDIN: ✓ Input #1 sent successfully
```

But NO corresponding output on stdout/stderr - the STDOUT/STDERR log lines never appear.

## Root Cause

**Claude CLI requires a PTY (pseudoterminal) to operate interactively.**

The current Rust implementation in `embedded_claude/process.rs` spawns Claude with simple piped stdin/stdout:

```rust
let mut cmd = Command::new("claude");
cmd.stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .stdin(Stdio::piped());
```

This approach does NOT work for Claude CLI because:

1. **Claude CLI is interactive** - designed for terminal use with ANSI codes, prompts, etc.
2. **Simple pipes don't provide TTY features** - Claude detects it's not in a terminal and behaves differently
3. **Test confirms:** Running `echo "Hello" | claude` hangs indefinitely (timeout after 10s)

## Evidence

### 1. PTY Wrapper Exists

The file `apps/desktop/wrappers/pty-claude-wrapper.js` exists and uses `node-pty`:

```javascript
this.ptyProcess = pty.spawn(shell, [], {
  name: 'xterm-256color',
  cols: 120,
  rows: 30,
  cwd: process.cwd(),
  env: {
    ...process.env,
    AGENTMUX_INSTANCE_NAME: this.instanceName,
    TERM: 'xterm-256color',
  }
});

// Send initial command to start Claude
setTimeout(() => {
  this.ptyProcess.write('claude\r');
  // ...
}, 1000);
```

This wrapper spawns a shell in a PTY, then starts Claude within that PTY.

### 2. Process is Running But Silent

```bash
$ tasklist | grep claude
claude.exe  234752 Console  1  289,208 K
```

PID 234752 is running, consuming memory, but producing NO output.

### 3. Direct Test Fails

```bash
$ echo "Hello" | claude
[hangs indefinitely - timeout after 10s]
```

Claude doesn't respond to piped input.

## Solution

**Replace the Rust process spawning with PTY support.**

### Option A: Use Rust PTY Library (Recommended for long-term)

Use a Rust PTY library like `portable-pty`:

```rust
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

let pty_system = native_pty_system();
let pair = pty_system.openpty(PtySize {
    rows: 30,
    cols: 120,
    pixel_width: 0,
    pixel_height: 0,
})?;

let mut cmd = CommandBuilder::new("claude");
cmd.cwd(workspace_path);

let child = pair.slave.spawn_command(cmd)?;
```

### Option B: Use Node.js PTY Wrapper (Quick fix)

Call the existing `pty-claude-wrapper.js` from Rust instead of spawning Claude directly:

```rust
let mut cmd = Command::new("node");
cmd.arg("wrappers/pty-claude-wrapper.js")
   .arg(&instance_name)
   .arg(&ws_port.to_string());
```

The wrapper handles PTY and already implements message watching, WebSocket server, etc.

### Option C: Spawn Shell + Claude (Middle ground)

Spawn a PTY shell, then send `claude\r` command to it:

```rust
// Windows: spawn powershell/cmd in PTY
// Linux/macOS: spawn bash in PTY
// Then write "claude\r" to the PTY stdin
```

## Recommended Immediate Action

**Use Option B** - Call the existing Node.js wrapper:

1. It's already implemented and tested
2. Handles PTY correctly
3. Already has message watching
4. Works cross-platform (Windows/Linux/macOS)
5. Can transition to Option A later for full Rust implementation

## Implementation Steps (Option B)

1. **Modify `embedded_claude/process.rs`:**
   - Change `spawn_claude_process()` to spawn `node wrappers/pty-claude-wrapper.js`
   - Keep WebSocket communication the same (wrapper provides this)
   - Remove direct stdin/stdout/stderr handling (wrapper handles this)

2. **Update WebSocket protocol:**
   - Wrapper sends JSON messages: `{ type: "output", data: "..." }`
   - UI needs to parse JSON instead of raw text

3. **Test:**
   - Spawn agent
   - Send input
   - Verify output appears in DebugConsole

## Files to Modify

- `apps/desktop/src-tauri/src/embedded_claude/process.rs` - Change spawn logic
- `apps/desktop/src-tauri/src/embedded_claude/websocket.rs` - Parse JSON messages
- `apps/desktop/src/components/AgentsManager.tsx` - Handle JSON protocol (if needed)

## Alternative: Full Rust PTY Implementation (Option A)

For a pure Rust solution:

1. Add dependency:
   ```toml
   [dependencies]
   portable-pty = "0.8"
   ```

2. Rewrite `embedded_claude/process.rs` to use PTY

3. Maintain all existing logging and WebSocket infrastructure

4. Benefits:
   - No Node.js dependency
   - Better performance
   - Single runtime
   - Easier deployment

5. Trade-offs:
   - More complex implementation
   - Platform-specific PTY handling
   - Need to reimplement message watching in Rust

## Conclusion

The comprehensive logging we added in v0.3.11 successfully identified the exact point of failure: **stdin is sent, but Claude never responds on stdout/stderr because it requires a PTY, not simple pipes.**

**Next PR should implement Option B (use Node.js wrapper) as immediate fix, with Option A (Rust PTY) as follow-up for clean architecture.**

---

## Related Files

- Working implementation: `D:\Code\WebProjects\agentmux\apps\desktop\wrappers\pty-claude-wrapper.js`
- Current (broken) implementation: `D:\Code\WebProjects\agentmux\apps\desktop\src-tauri\src\embedded_claude\process.rs`
- Debug logs: DebugConsole UI (v0.3.11+)

## References

- node-pty: https://github.com/microsoft/node-pty
- portable-pty (Rust): https://github.com/wez/wezterm/tree/main/pty
- PTY vs Pipes: https://stackoverflow.com/questions/52954248/what-is-a-pseudo-terminal-pty
