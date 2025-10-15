# AgentMux Desktop v0.3.1

**Release Date:** 2025-10-14
**Type:** Logging & Diagnostics Enhancement

---

## 🎯 What's New in v0.3.1

### 🔍 **Comprehensive Logging Added**

This release adds **heavy logging** throughout the entire WebSocket → stdin pipeline to diagnose UI communication issues.

**Every step is now logged:**

1. **WebSocket Message Receipt**
   ```
   [WS:127.0.0.1:xxxxx] ← Received text message #1: 'hello' (5 bytes)
   [WS:127.0.0.1:xxxxx] → Forwarding to stdin channel...
   [WS:127.0.0.1:xxxxx] ✓ Successfully sent to stdin channel
   ```

2. **stdin Channel Processing**
   ```
   [instance:stdin] → Sending input #1 (5 bytes)
   [instance:stdin] ✓ Input #1 sent successfully
   ```

3. **stdout/stderr Streaming**
   ```
   [instance:stdout] Line #1 - Broadcasting to 1 peer(s)
   [instance:stdout] Response from Claude...
   ```

### 🔧 **Bug Fixes**

- ✅ Fixed dialog ACL permissions (resolves "Command plugin:dialog|open not allowed by ACL" error)
- ✅ Added missing dialog permissions: `dialog:allow-open`, `dialog:allow-save`, `dialog:default`

### 🧪 **Automated Tests Added**

New integration test suite: `websocket_stdin_test.rs`

**Tests:**
1. Single message forwarding
2. Multiple sequential messages
3. Closed channel error handling

**Run tests:**
```bash
cd apps/desktop/src-tauri
cargo test --test websocket_stdin_test -- --nocapture
```

---

## 📥 Download

### Portable EXE (Recommended for Testing)
**File:** `agentmux-desktop-v0.3.1-portable.exe`
- **Size:** ~19MB
- No installation required
- Run from any location
- Can run multiple versions simultaneously

### MSI Installer
**File:** `agentmux-desktop-v0.3.1-installer.msi`
- **Size:** ~6.2MB
- System-wide installation
- Start menu integration
- WebView2 auto-installed

---

## 🐛 Debugging with v0.3.1

### If "I type input, see nothing in logs"

With v0.3.1's comprehensive logging, you'll now see **exactly where** the flow breaks:

**Step 1: Check WebSocket Connection**
```
Expected: WebSocket server listening on 127.0.0.1:9999
Expected: [WS:127.0.0.1:xxxxx] ✓ WebSocket connected
```

**Step 2: Check Message Receipt**
```
Expected: [WS:127.0.0.1:xxxxx] ← Received text message #1: 'your message' (X bytes)
Expected: [WS:127.0.0.1:xxxxx] → Forwarding to stdin channel...
```

**Step 3: Check Channel Forwarding**
```
Expected: [WS:127.0.0.1:xxxxx] ✓ Successfully sent to stdin channel

If Error: [WS:127.0.0.1:xxxxx] ✗ Failed to forward to stdin channel: <error>
          [WS:127.0.0.1:xxxxx] ✗ This usually means the stdin handler task has stopped
```

**Step 4: Check stdin Handler**
```
Expected: [instance:stdin] → Sending input #1 (X bytes)
Expected: [instance:stdin] ✓ Input #1 sent successfully
```

**Step 5: Check Response**
```
Expected: [instance:stdout] Line #1 - Broadcasting to 1 peer(s)
Expected: [instance:stdout] <Claude's response>
```

---

## 🔄 Upgrading from v0.3.0

### Key Changes
- **Same features** as v0.3.0
- **Added:** Comprehensive logging at every pipeline step
- **Added:** Automated test suite
- **Fixed:** Dialog ACL permissions

### Why Upgrade?
- **Diagnose issues** - See exactly where communication breaks
- **Better debugging** - Every WebSocket message is logged
- **More stable** - Dialog permissions now work correctly

---

## 🚀 Quick Start

### 1. Run the Portable EXE
```bash
# No installation needed!
./agentmux-desktop-v0.3.1-portable.exe
```

### 2. Open Console/Terminal
- Keep the terminal open where you launched the app
- All logs will appear in this terminal

### 3. Test Communication
1. Click "Spawn Agent"
2. Type a message
3. Click "Send"
4. **Watch the logs** - you'll see every step

### 4. Share Logs for Support
If issues occur, copy the terminal output showing:
- What logs appeared
- What logs are missing
- Any error messages

---

## 📋 Technical Details

### Build Information
- **Version:** 0.3.1
- **Build Date:** 2025-10-14
- **Rust Toolchain:** Stable
- **Tauri:** 2.2
- **Target:** Windows x64

### Files Modified (from v0.3.0)
- `capabilities/default.json` - Dialog ACL permissions
- `embedded_claude.rs` - Enhanced WebSocket logging
- `websocket_stdin_test.rs` - New test suite (216 lines)

### Compilation Status
✅ Success (warnings only, no errors)

---

## 🔗 Related Versions

- **v0.3.0** - Initial WebSocket stdin forwarding fix
- **v0.3.1** - Added comprehensive logging (this release)

---

## 📝 Known Issues

None specific to v0.3.1. If you encounter issues, the enhanced logging will help diagnose them.

---

## 💡 Tips

### Running Multiple Versions
Since each version has its own folder, you can:
```bash
# Terminal 1
cd releases/v0.3.0
./agentmux-desktop-v0.3.0-portable.exe

# Terminal 2
cd releases/v0.3.1
./agentmux-desktop-v0.3.1-portable.exe
```

Both will run independently!

### Viewing Logs
- Logs only appear in the **terminal where you launched the app**
- Use `| tee output.log` to save logs to a file:
  ```bash
  ./agentmux-desktop-v0.3.1-portable.exe | tee logs.txt
  ```

---

## 🆘 Support

If you encounter issues:

1. **Capture logs** from the terminal
2. Note which expected log messages are missing (see "Debugging" section above)
3. Share the logs for support

The comprehensive logging in v0.3.1 makes it much easier to diagnose issues!

---

**Installation:** No special setup required - just run the portable exe!
