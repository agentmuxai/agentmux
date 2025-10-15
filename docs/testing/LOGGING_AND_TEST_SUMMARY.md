# WebSocket stdin Logging & Testing Implementation

**Date:** 2025-10-14
**Agent:** AgentX
**Version:** v0.3.0

---

## Summary

Fixed dialog ACL permissions and added comprehensive logging throughout the WebSocket → stdin pipeline to debug UI communication issues. Created automated integration test suite.

---

## Changes Made

### 1. Fixed Dialog ACL Permissions ✅

**File:** `apps/desktop/src-tauri/capabilities/default.json`

Added missing dialog permissions:
```json
"dialog:allow-open",
"dialog:allow-save",
"dialog:default"
```

**Fix:** Resolves "Command plugin:dialog|open not allowed by ACL" error

---

### 2. Enhanced WebSocket Message Logging ✅

**File:** `apps/desktop/src-tauri/src/embedded_claude.rs` (lines 277-290)

**Before:**
```rust
Ok(Message::Text(text)) => {
    println!("[WS:{}] ← Received text message #{}: {}", addr, incoming_count, text);
    if let Err(e) = stdin_tx.send(text.to_string()) {
        eprintln!("[WS:{}] ✗ Failed to forward to stdin: {}", addr, e);
    }
}
```

**After:**
```rust
Ok(Message::Text(text)) => {
    incoming_count += 1;
    println!("[WS:{}] ← Received text message #{}: '{}' ({} bytes)", addr, incoming_count, text, text.len());
    println!("[WS:{}] → Forwarding to stdin channel...", addr);

    match stdin_tx.send(text.to_string()) {
        Ok(_) => {
            println!("[WS:{}] ✓ Successfully sent to stdin channel", addr);
        }
        Err(e) => {
            eprintln!("[WS:{}] ✗ Failed to forward to stdin channel: {}", addr, e);
            eprintln!("[WS:{}] ✗ This usually means the stdin handler task has stopped", addr);
        }
    }
}
```

**Improvements:**
- Shows message content and byte count
- Logs forwarding attempt
- Distinguishes success vs failure
- Explains error condition

---

### 3. Existing Comprehensive Logging (Verified) ✅

The following logging was already present and comprehensive:

#### stdin Handler (lines 360-385)
```rust
println!("[{}:stdin] Stdin handler task started", instance_name);
println!("[{}:stdin] → Sending input #{} ({} bytes)", instance_name, input_count, input.len());
println!("[{}:stdin] ✓ Input #{} sent successfully", instance_name, input_count);
```

#### stdout/stderr Streaming (lines 316-360)
```rust
println!("[{}:{}] Stream monitoring task started", instance_name, stream_type);
println!("[{}:{}] Line #{} - Broadcasting to {} peer(s)", instance_name, stream_type, line_count, peer_count);
println!("[{}:{}] Line #{} - No peers connected, data not broadcast", instance_name, stream_type, line_count);
```

#### Process Initialization (lines 90-178)
```rust
println!("[embedded_claude] {} - Process spawned with PID: {}", instance_name, pid);
println!("[embedded_claude] {} - stdout captured", instance_name);
println!("[embedded_claude] {} - stderr captured", instance_name);
println!("[embedded_claude] {} - stdin captured", instance_name);
println!("[embedded_claude] {} - Starting WebSocket server on port {}...", instance_name, ws_port);
```

---

### 4. Automated Integration Test Suite ✅

**File:** `apps/desktop/src-tauri/tests/websocket_stdin_test.rs` (new file)

**Test Coverage:**

#### Test 1: `test_websocket_to_stdin_forwarding`
- Creates stdin channel
- Starts mock WebSocket server
- Connects as client (simulates UI)
- Sends test message: `"hello claude\n"`
- Verifies message received in stdin channel
- Validates message content matches
- Confirms ACK response

**Expected Output:**
```
=== Starting WebSocket → stdin forwarding test ===
[TEST] ✓ Created stdin channel
[TEST] Starting mock WebSocket server on port 9998...
[TEST] ✓ Mock server listening on port 9998
[TEST] ✓ Accepted connection from 127.0.0.1:xxxxx
[TEST] ✓ WebSocket handshake completed
[TEST] Sending test message: 'hello claude'
[TEST] ✓ Message sent via WebSocket
[TEST] ← Received message #1: 'hello claude'
[TEST] → Forwarding to stdin channel...
[TEST] ✓ Successfully forwarded to stdin channel
[TEST] ✓ stdin channel received: 'hello claude'
[TEST] ✓ Message content matches!
[TEST] ✓ Received ACK: ACK: hello claude
[TEST] ✓ WebSocket closed cleanly
=== Test completed successfully! ===
```

#### Test 2: `test_multiple_messages_sequentially`
- Sends 3 messages sequentially
- Verifies all received in order
- Tests message queuing

#### Test 3: `test_stdin_channel_closed_handling`
- Tests error handling when stdin channel is closed
- Verifies proper error reporting

**Run Tests:**
```bash
cd apps/desktop/src-tauri
cargo test --test websocket_stdin_test -- --nocapture
```

---

## Logging Flow Diagram

```
┌─────────────────┐
│   UI Frontend   │
│  (SolidJS)      │
└────────┬────────┘
         │ send("hello\n")
         ↓
┌─────────────────┐
│   WebSocket     │  [WS:127.0.0.1:xxxxx] ← Received text message #1: 'hello' (6 bytes)
│   Connection    │  [WS:127.0.0.1:xxxxx] → Forwarding to stdin channel...
└────────┬────────┘  [WS:127.0.0.1:xxxxx] ✓ Successfully sent to stdin channel
         │
         ↓
┌─────────────────┐
│  stdin_tx       │  (mpsc::UnboundedSender<String>)
│  Channel        │
└────────┬────────┘
         │
         ↓
┌─────────────────┐
│  stdin_rx       │  [instance:stdin] → Sending input #1 (6 bytes)
│  Handler Task   │  [instance:stdin] ✓ Input #1 sent successfully
└────────┬────────┘
         │
         ↓
┌─────────────────┐
│ Claude Process  │  (claude code subprocess)
│     stdin       │
└─────────────────┘
```

---

## Debugging Procedure

When user reports "I type input, click send, see nothing in logs":

### 1. Check WebSocket Connection
**Expected Logs:**
```
WebSocket server listening on 127.0.0.1:9999
[WS:127.0.0.1:xxxxx] ✓ WebSocket connected
[WS:127.0.0.1:xxxxx] Starting incoming message loop...
```

**If Missing:** WebSocket server failed to start or client didn't connect

---

### 2. Check Message Receipt
**Expected Logs:**
```
[WS:127.0.0.1:xxxxx] ← Received text message #1: 'hello' (5 bytes)
[WS:127.0.0.1:xxxxx] → Forwarding to stdin channel...
```

**If Missing:**
- Frontend not sending messages via WebSocket
- Message format incorrect
- WebSocket connection dropped

---

### 3. Check Channel Forwarding
**Expected Logs:**
```
[WS:127.0.0.1:xxxxx] ✓ Successfully sent to stdin channel
```

**If Missing (Error Instead):**
```
[WS:127.0.0.1:xxxxx] ✗ Failed to forward to stdin channel: <error>
[WS:127.0.0.1:xxxxx] ✗ This usually means the stdin handler task has stopped
```

**Action:** Check if stdin handler task crashed or process exited

---

### 4. Check stdin Handler
**Expected Logs:**
```
[instance:stdin] Stdin handler task started
[instance:stdin] → Sending input #1 (5 bytes)
[instance:stdin] ✓ Input #1 sent successfully
```

**If Missing:**
- stdin handler task never started
- stdin handler crashed before receiving message

---

### 5. Check Claude Process
**Expected Logs:**
```
[embedded_claude] instance - Process spawned with PID: 12345
[embedded_claude] instance - stdout captured
[embedded_claude] instance - stderr captured
[embedded_claude] instance - stdin captured
```

**If Missing:** Claude process failed to start

---

## Build Information

**Version:** 0.3.0
**Build Date:** 2025-10-14 07:50 PST
**Build Location:**
- Portable EXE: `D:\Code\WebProjects\agentmux\apps\desktop\src-tauri\target\release\agentmux.exe`
- MSI Installer: `D:\Code\WebProjects\agentmux\apps\desktop\src-tauri\target\release\bundle\msi\AgentMux Desktop_0.3.0_x64_en-US.msi`

**Release Location:**
- `D:\Code\WebProjects\agentmux\releases\v0.3.0\agentmux-desktop-v0.3.0-portable.exe`
- `D:\Code\WebProjects\agentmux\releases\v0.3.0\agentmux-desktop-v0.3.0-installer.msi`

**Compilation Status:** ✅ Success (warnings only, no errors)

---

## Testing Instructions

### Manual Test
1. Run `releases/v0.3.0/agentmux-desktop-v0.3.0-portable.exe`
2. Click "Spawn Agent"
3. Type message in input field
4. Click "Send"
5. **Check console/terminal for logs** - you should see detailed logging at every step

### Automated Test
```bash
cd apps/desktop/src-tauri
cargo test --test websocket_stdin_test -- --nocapture
```

Expected: All 3 tests pass with detailed logging output

---

## Next Steps

1. Run the updated v0.3.0 build
2. Attempt to send a message through the UI
3. **Capture and review the console logs** - every step is now logged
4. Identify where the flow breaks (if it does)
5. Use the "Debugging Procedure" section above to diagnose

---

## Files Modified

- `apps/desktop/src-tauri/capabilities/default.json` - Added dialog ACL permissions
- `apps/desktop/src-tauri/src/embedded_claude.rs` - Enhanced WebSocket message logging
- `apps/desktop/src-tauri/tests/websocket_stdin_test.rs` - New integration test suite (216 lines)

---

## Compilation Warnings (Non-Critical)

- Unused variable: `app_handle` in ipc/server.rs
- Unused import: `std::os::windows::process::CommandExt`
- Unreachable pattern in ipc/server.rs
- Dead code: `WebSocketSink`, `AgentProcesses`, etc.

None of these affect functionality.

---

**Status:** ✅ Complete - Ready for testing with comprehensive logging
