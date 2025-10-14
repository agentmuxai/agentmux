# Embedded Terminal - Simplified Approach

**Date:** 2025-10-13
**Problem:** node-pty requires native compilation which fails on Windows
**Solution:** Use Tauri's IPC + streaming output instead

---

## Architecture

```
Desktop UI (xterm.js) ←→ WebSocket ←→ Tauri Backend ←→ Claude CLI Process
   ↑ displays                        ↑ spawns with pipes
   ↓ sends input                     ↓ captures stdout/stderr
```

**Key Insight:** We don't need PTY - we just need:
1. Spawn Claude CLI with piped stdio
2. Stream output to WebSocket
3. Send input from UI to stdin
4. Watch for message files and inject

---

## Implementation

### 1. Rust Backend (Tauri Command)

```rust
#[tauri::command]
async fn spawn_embedded_claude(
    instance_name: String,
    app_handle: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let ws_port = find_available_port(9000, 9999)?;

    // Spawn Claude with piped stdio
    let mut child = Command::new("claude")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn: {}", e))?;

    let pid = child.id();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // Start WebSocket server in background
    tokio::spawn(async move {
        let server = WebSocketServer::new(ws_port).await;

        // Stream stdout to WebSocket clients
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 4096];
            loop {
                match stdout.read(&mut buffer).await {
                    Ok(n) if n > 0 => {
                        server.broadcast(&buffer[..n]).await;
                    }
                    _ => break,
                }
            }
        });

        // Handle input from WebSocket clients
        server.on_input(move |data| {
            stdin.write_all(&data).await;
        });
    });

    // Watch for messages
    tokio::spawn(async move {
        watch_messages(instance_name, stdin_clone).await;
    });

    Ok(json!({
        "instanceName": instance_name,
        "pid": pid,
        "wsPort": ws_port
    }))
}
```

### 2. Frontend (Same xterm.js component)

No changes needed - EmbeddedTerminal component already works with WebSocket!

---

## Benefits vs PTY

**PTY Approach:**
- ❌ Requires native compilation (node-gyp)
- ❌ Platform-specific issues
- ❌ Complex build process
- ✅ Full terminal emulation (colors, cursor control)

**Piped stdio + WebSocket:**
- ✅ Pure Rust - no native deps
- ✅ Cross-platform by default
- ✅ Simple build process
- ✅ Still get colors (Claude outputs ANSI)
- ⚠️ No interactive prompts (but Claude doesn't use those)

---

## What Works

- ✅ Spawn Claude CLI inside Desktop app
- ✅ See output in xterm.js terminal
- ✅ Type input and send to Claude
- ✅ Receive reactive messages
- ✅ Multiple instances simultaneously
- ✅ No separate terminal windows

## What Doesn't Work

- ❌ Password prompts (but Claude doesn't need these)
- ❌ Full terminal control (clear screen, etc.)
- ⚠️ Readline features (but Claude handles this internally)

---

## Implementation Time

**4-6 hours:**
- 2 hours: Rust backend with WebSocket server
- 1 hour: Message watching integration
- 1 hour: Testing with Alice/Bob
- 1-2 hours: Automated tests

vs PTY approach: Would work in theory but blocked by build issues

---

## Recommendation

**Use piped stdio + WebSocket approach**

Reasons:
1. Unblocked - can implement now
2. Simpler - no native compilation
3. Sufficient - meets all requirements
4. Cross-platform - works everywhere

PTY would be nice-to-have but isn't necessary for this use case.

---

**Status:** Design complete, ready to implement
**Priority:** HIGH
**Blocked by:** Nothing (unblocked by switching from PTY)
