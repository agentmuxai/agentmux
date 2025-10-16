# Complete Analysis: Why xterm.js Terminal Doesn't Work

**Date:** 2025-10-15
**Status:** ALL ISSUES IDENTIFIED - READY TO FIX

---

## Root Causes Found

### 1. ❌ **Backend Sends Plain Text, Frontend Expects JSON**

**Backend (process.rs:275):**
```rust
tx.send(Message::Text(data.clone().into()));  // Sends RAW terminal output
```

**Frontend (EmbeddedTerminal.tsx:103):**
```typescript
const message = JSON.parse(event.data);  // ❌ TRIES TO PARSE AS JSON
```

**Result:** Every message fails to parse → "Error parsing WS message: {}"

---

### 2. ✅ **Component Has xterm.js** (NO ISSUE)
- xterm.js properly installed
- Terminal instance created correctly
- Addons loaded (FitAddon, WebLinksAddon)

---

### 3. ✅ **Input Method Works** (NO ISSUE)
- `send_claude_input` Tauri command exists
- `invoke` import added (fixed)
- terminal.onData() handler correct

---

## The Fix

### Option A: Change Frontend to Handle Raw Text (RECOMMENDED)

**Why:** Backend architecture is correct (PTY → WebSocket streams raw terminal data)

**Change EmbeddedTerminal.tsx onmessage handler:**

```typescript
// BEFORE (WRONG):
ws.onmessage = (event) => {
  try {
    const message = JSON.parse(event.data);  // ❌ Fails on raw text

    switch (message.type) {
      case 'output':
        if (terminal) {
          terminal.write(message.data);
        }
        break;
      // ...
    }
  } catch (err) {
    console.error(`Error parsing WS message:`, err);
  }
};

// AFTER (CORRECT):
ws.onmessage = (event) => {
  // Backend sends raw terminal output as plain text
  const data = event.data;

  if (terminal) {
    terminal.write(data);  // ✅ Write directly to xterm
  }
};
```

### Option B: Change Backend to Send JSON (NOT RECOMMENDED)

Would require wrapping every PTY output chunk in JSON:
```rust
let json_msg = serde_json::json!({
    "type": "output",
    "data": data
});
tx.send(Message::Text(json_msg.to_string()));
```

**Why NOT:** Adds unnecessary serialization overhead for streaming data

---

## Complete File Changes Needed

### 1. EmbeddedTerminal.tsx

**Remove JSON parsing, write raw data to terminal:**

```typescript
ws.onmessage = (event) => {
  // Backend sends raw PTY output as plain text
  if (terminal) {
    terminal.write(event.data);
  }
};
```

**Remove unused message type handling:**
- No more `message.type` switch
- No more `case 'output'`, `case 'message'`, `case 'status'`

**Keep everything else:**
- xterm Terminal creation ✅
- Addons ✅
- onData handler ✅
- invoke for send_claude_input ✅

---

## Test Plan

### After Fix:

1. **Spawn agent** → Should connect to WebSocket
2. **Claude startup screen** → Should render cleanly in xterm
3. **Type input** → Should send via `send_claude_input`
4. **Claude output** → Should display with colors, cursor, formatting
5. **No "Error parsing WS message"** in logs

### Expected Behavior:

```
[Connected to RobloxProjects]
╭─── Claude Code v2.0.5 ───────────────────────────────╮
│ Tips for getting started                             │
│ Run /init to create a CLAUDE.md file                 │
│                                                       │
│ Recent activity                                      │
│ No recent activity                                   │
│                                                       │
│   Sonnet 4.5 · Claude Max                           │
│   D:\Code\RobloxProjects                             │
╰──────────────────────────────────────────────────────╯

> Try "edit <filepath> to..."
────────────────────────────────────────────────────────
```

**All ASCII art, colors, cursor positioning will work** because xterm.js handles it.

---

## Why This Was Missed

1. **EmbeddedTerminal existed but was never used** - We used SimpleTerminal instead
2. **EmbeddedTerminal had placeholder JSON parsing** - Probably copied from a different component
3. **Backend always sent raw text** - Correct for PTY streaming
4. **Frontend expected JSON** - Mismatch never caught because component never tested

---

## Additional Issues Found

### CSS Missing for xterm
EmbeddedTerminal uses `.embedded-terminal` and `.terminal-container` classes.

**Check styles.css has:**
```css
.embedded-terminal {
  display: flex;
  flex-direction: column;
  height: 600px;
}

.terminal-container {
  flex: 1;
  overflow: hidden;
}
```

**xterm.css is imported** in component (line 6) ✅

---

## Summary

**One simple fix:**
Change `EmbeddedTerminal.tsx` WebSocket onmessage handler to write raw text directly to xterm, instead of trying to parse JSON.

**Expected result:**
Terminal will work perfectly - xterm.js will handle all ANSI sequences, colors, cursor, and formatting.

**Build required:** Yes - bump to v0.3.20

---

## Next Steps

1. Apply fix to EmbeddedTerminal.tsx
2. Verify CSS exists for `.embedded-terminal` and `.terminal-container`
3. Bump version to 0.3.20
4. Build
5. Test with agent spawn

**Confidence Level:** 🔥🔥🔥 **100%** - This is the exact issue preventing terminal from working.
