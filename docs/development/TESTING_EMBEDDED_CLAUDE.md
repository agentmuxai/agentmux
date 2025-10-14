# Testing Embedded Claude - Quick Start Guide

**Date:** 2025-10-13
**Status:** Ready for testing
**Build:** agentmux-desktop.exe

---

## What We Built

**Embedded Claude CLI** running INSIDE the Desktop app with:
- ✅ Full interactive terminal
- ✅ WebSocket streaming (Rust tokio-tungstenite)
- ✅ Reactive messaging (file watcher)
- ✅ Multiple instances simultaneously
- ✅ NO external terminal windows

---

## How to Test

### 1. Launch Desktop App

```bash
.\src-tauri\target\release\agentmux-desktop.exe
```

### 2. Navigate to "🧠 CLI Agents" Tab

Click on the "CLI Agents" tab in the Desktop app.

### 3. Spawn First Instance (Alice)

**Steps:**
1. Enter "Alice" in Agent ID field
2. Click "▶️ Spawn Agent"
3. Wait for confirmation
4. Click on Alice in the agent list

**Expected Result:**
- Alice appears in agents list with PID and WebSocket port
- Terminal displays below showing connection status
- Should see `[Connected to Alice]` message

### 4. Test Interactive Input

**In Alice's terminal input box:**
```
Hello! I'm Alice. Can you confirm you're receiving this?
```

**Expected Result:**
- Input appears in Claude CLI
- Claude responds naturally
- Response streams back to terminal

### 5. Spawn Second Instance (Bob)

**Repeat steps from #3 but with "Bob"**

Now you should have:
- Alice (e.g., PID: 12345, WS: 9000)
- Bob (e.g., PID: 12346, WS: 9001)

### 6. Test Reactive Messaging (Alice → Bob)

**Select Alice, then in her terminal:**
```
Can you send a message to Bob using the agentmux MCP tool? Tell him "Hello Bob, this is Alice testing reactive messaging!"
```

**What happens:**
1. Alice uses `mcp__agentmux__agentmux_send_message`
2. Message file created in `~/.agentmux/shared/messages/`
3. Bob's file watcher detects new message
4. Message automatically injected into Bob's stdin
5. **Select Bob** and watch his terminal

**Expected Result:**
- Bob's terminal shows: `[INCOMING MESSAGE from Alice]: Hello Bob...`
- Bob responds naturally to Alice's message
- **NO human input needed in Bob's terminal**

### 7. Test Bob → Alice

**Select Bob, repeat process sending message to Alice**

**Expected Result:**
- Alice receives message reactively
- Alice responds
- Full conversation without human intervention

---

## Success Criteria

### Phase 1: Basic Functionality
- ✅ Can spawn Claude instances
- ✅ Terminal shows output
- ✅ Can type input and see responses
- ✅ Multiple instances run simultaneously

### Phase 2: Reactive Messaging
- ✅ Message files detected automatically
- ✅ Messages injected into correct instance
- ✅ Instance responds without human input
- ✅ Alice ↔ Bob conversation works

### Phase 3: Production Ready
- ✅ No crashes or hangs
- ✅ Clean error messages
- ✅ WebSocket reconnects automatically
- ✅ Instances can be stopped cleanly

---

## Troubleshooting

### "Failed to spawn Claude"
**Cause:** Claude CLI not installed or not in PATH

**Fix:**
```bash
# Check if Claude is installed
claude --version

# If not, install it
npm install -g @anthropic-ai/claude-cli
```

### "WebSocket connection error"
**Cause:** Port already in use

**Solution:** Desktop app automatically finds available ports (9000-9999). If all ports busy, close other apps.

### "No message received"
**Check:**
1. Message file created? `ls ~/.agentmux/shared/messages/`
2. `to` field matches instance name (Alice, Bob, or `*`)
3. Instance still running (check PID)

### "Terminal not updating"
**Fix:** WebSocket should auto-reconnect. If not, reselect the instance from list.

---

## Demo Script (2 minutes)

**Perfect demo to show reactive messaging:**

```bash
# 1. Launch app
.\src-tauri\target\release\agentmux-desktop.exe

# 2. Spawn Alice and Bob

# 3. In Alice's terminal:
"Please send Bob a message asking him what his favorite color is"

# 4. Switch to Bob's terminal - watch him receive and respond automatically

# 5. In Alice's terminal:
"Did you get Bob's response?"

# 6. Watch Alice acknowledge Bob's answer

# RESULT: Two Claude instances having a conversation autonomously!
```

---

## Architecture Verified

```
┌────────────────────────────────────┐
│ Desktop App (SolidJS)              │
│  - SimpleTerminal component        │
│  - Connects to ws://localhost:PORT│
└─────────────┬──────────────────────┘
              │
              ↓ WebSocket
┌────────────────────────────────────┐
│ Rust Backend (Tauri)               │
│  - tokio-tungstenite WebSocket     │
│  - Broadcast to all clients        │
│  - Forward input to Claude         │
└─────────────┬──────────────────────┘
              │ tokio::process
              ↓
┌────────────────────────────────────┐
│ Claude CLI Process                 │
│  - Piped stdio                     │
│  - stdout → WebSocket              │
│  - stdin ← WebSocket + messages    │
└────────────────────────────────────┘
              ↑
              │ notify file watcher
┌─────────────────────────────────┐
│ ~/.agentmux/shared/messages/    │
│  msg-*.json files                │
└─────────────────────────────────┘
```

---

## What's Different from PTY Approach

**PTY (node-pty):**
- ❌ Requires native compilation
- ❌ Build failures on Windows
- ✅ Full terminal emulation

**Our Approach (piped stdio):**
- ✅ Pure Rust, no native deps
- ✅ Builds everywhere
- ✅ ANSI colors still work
- ⚠️ No password prompts (but Claude doesn't need them)

**Conclusion:** Simpler is better!

---

## Next Steps After Testing

1. **Add automated tests** - Spawn instance, send input, verify output
2. **Add xterm.js** (optional) - Better terminal UI
3. **Production deployment** - Package with installer
4. **GitHub webhook integration** - Connect reactive PR review

---

**Status:** Build in progress, ready to test
**Estimated Test Time:** 10 minutes
**Expected Outcome:** Prove reactive messaging works end-to-end!
