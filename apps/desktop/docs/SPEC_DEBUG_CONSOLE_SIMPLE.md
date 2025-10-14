# AgentMux Desktop - Simple Debug Console

**Date:** 2025-10-13
**Purpose:** Quick debug view at bottom of window
**Status:** Ready for implementation

---

## What It Is

A **simple text console** at the bottom of the app window that shows **application errors only**:
- Frontend JavaScript errors (SolidJS component errors, exceptions)
- Backend Rust errors (panic, failed operations)
- Application warnings (not agent messages or bus traffic)

Think of it like opening DevTools but built into the app. **NOT for agent messages** - those go in the Messages tab.

---

## Where It Goes

```
┌─────────────────────────────────┐
│ AgentMux Header                 │
├─────────────────────────────────┤
│                                  │
│ Main Content (Tabs)             │
│                                  │
├─────────────────────────────────┤
│ Status Bar                      │
├─────────────────────────────────┤
│ [▼ Debug Console] [Clear] [Copy]│ ← Collapsible header
├─────────────────────────────────┤
│ 09:41:25.789 [ERR] TypeError... │ ← APP ERRORS ONLY
│ 09:41:26.123 [RUST] Bus failed  │ ← NOT agent messages
│ 09:41:27.456 [WARN] Component...│ ← NOT bus traffic
│ ...                              │
└─────────────────────────────────┘
        ↑ NEW (below status bar)

What goes here:
✅ JavaScript exceptions
✅ SolidJS component errors
✅ Rust panic/errors
✅ Tauri command failures
✅ Application warnings

What does NOT go here:
❌ Agent messages (use Messages tab)
❌ Bus message traffic (use Messages tab)
❌ console.log debug output
❌ Agent status updates
```

---

## Simple Implementation

### 1. Frontend Component

**File:** `src/components/DebugConsole.tsx`

```typescript
import { createSignal, For, Show } from 'solid-js';

interface DebugLog {
  time: string;
  prefix: string;
  message: string;
}

export function DebugConsole() {
  const [logs, setLogs] = createSignal<DebugLog[]>([]);
  const [collapsed, setCollapsed] = createSignal(false);

  // Intercept ERRORS ONLY (not regular console.log)
  const originalError = console.error;
  const originalWarn = console.warn;

  console.error = (...args) => {
    originalError.apply(console, args);
    addLog('[ERR]', args.join(' '));
  };

  console.warn = (...args) => {
    originalWarn.apply(console, args);
    addLog('[WARN]', args.join(' '));
  };

  // Catch uncaught errors
  window.addEventListener('error', (event) => {
    addLog('[ERR]', `${event.message} at ${event.filename}:${event.lineno}`);
  });

  // Catch unhandled promise rejections
  window.addEventListener('unhandledrejection', (event) => {
    addLog('[ERR]', `Unhandled promise rejection: ${event.reason}`);
  });

  function addLog(prefix: string, message: string) {
    const time = new Date().toLocaleTimeString('en-US', {
      hour12: false,
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      fractionalSecondDigits: 3,
    });

    setLogs([{ time, prefix, message }, ...logs()].slice(0, 100));
  }

  function handleClear() {
    setLogs([]);
  }

  function handleCopy() {
    const text = logs()
      .reverse()
      .map(l => `${l.time} ${l.prefix} ${l.message}`)
      .join('\n');
    navigator.clipboard.writeText(text);
  }

  return (
    <div class="debug-console">
      <div class="debug-console-header">
        <button
          onClick={() => setCollapsed(!collapsed())}
          class="debug-console-toggle"
        >
          {collapsed() ? '▶' : '▼'} Debug Console ({logs().length})
        </button>
        <button onClick={handleClear} class="debug-console-btn">Clear</button>
        <button onClick={handleCopy} class="debug-console-btn">Copy</button>
      </div>

      <Show when={!collapsed()}>
        <div class="debug-console-content">
          <Show when={logs().length === 0} fallback={
            <For each={logs()}>
              {(log) => (
                <div class="debug-log-entry">
                  <span class="debug-time">{log.time}</span>
                  <span
                    class="debug-prefix"
                    classList={{
                      'debug-prefix-error': log.prefix === '[ERR]',
                      'debug-prefix-warn': log.prefix === '[WARN]',
                    }}
                  >
                    {log.prefix}
                  </span>
                  <span class="debug-message">{log.message}</span>
                </div>
              )}
            </For>
          }>
            <div class="debug-empty">No logs</div>
          </Show>
        </div>
      </Show>
    </div>
  );
}
```

### 2. Add to App Layout

**File:** `src/App.tsx`

```typescript
import { DebugConsole } from './components/DebugConsole';

// In render, after status bar:
return (
  <div class="app">
    <header>...</header>
    <main>...</main>
    <div class="status-bar">...</div>
    <DebugConsole />  {/* ← Add here */}
  </div>
);
```

### 3. Styling

**File:** `src/styles.css`

```css
/* Debug Console */
.debug-console {
  border-top: 2px solid #3a3a3a;
  background: #0a0a0a;
}

.debug-console-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem;
  background: #1a1a1a;
  border-bottom: 1px solid #2a2a2a;
}

.debug-console-toggle {
  background: none;
  border: none;
  color: #e0e0e0;
  font-family: monospace;
  font-size: 0.875rem;
  cursor: pointer;
  padding: 0.25rem 0.5rem;
}

.debug-console-toggle:hover {
  background: #2a2a2a;
}

.debug-console-btn {
  background: #2a2a2a;
  border: 1px solid #3a3a3a;
  color: #e0e0e0;
  padding: 0.25rem 0.75rem;
  border-radius: 3px;
  cursor: pointer;
  font-size: 0.75rem;
}

.debug-console-btn:hover {
  background: #3a3a3a;
}

.debug-console-content {
  height: 150px;
  overflow-y: auto;
  padding: 0.5rem;
  font-family: 'Courier New', monospace;
  font-size: 0.75rem;
  line-height: 1.4;
}

.debug-log-entry {
  display: flex;
  gap: 0.5rem;
  padding: 0.125rem 0;
  color: #e0e0e0;
}

.debug-time {
  color: #666;
  min-width: 95px;
}

.debug-prefix {
  color: #4a9eff;
  font-weight: bold;
  min-width: 50px;
}

.debug-prefix-error {
  color: #ef5350;
}

.debug-prefix-warn {
  color: #ff9800;
}

.debug-message {
  flex: 1;
  word-break: break-word;
}

.debug-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #666;
}
```

---

## That's It!

**3 steps:**
1. Create DebugConsole component (intercepts console)
2. Add `<DebugConsole />` to App.tsx
3. Add CSS

**Result:**
- All console.log/error/warn visible at bottom
- Collapsible with toggle
- Clear and Copy buttons
- Last 100 logs kept
- Simple and fast

---

## Optional: Backend Logs

If you want Rust logs too, add this minimal backend:

**File:** `src-tauri/src/main.rs`

```rust
use tauri::Manager;

// In main(), before run():
let app = tauri::Builder::default()
    .setup(|app| {
        let window = app.get_window("main").unwrap();

        // Log to frontend
        macro_rules! frontend_log {
            ($msg:expr) => {
                window.emit("debug_log", ($msg.to_string(),)).ok();
            };
        }

        // Example: log bus events
        info!("Backend started");
        frontend_log!("[BE] Backend started");

        Ok(())
    })
    // ... rest
```

**Then in DebugConsole.tsx:**

```typescript
import { listen } from '@tauri-apps/api/event';

onMount(async () => {
  await listen('debug_log', (event) => {
    const message = event.payload as string;
    addLog('[BE]', message);
  });
});
```

---

## Why This is Better

**vs Full Log Viewer:**
- ✅ 10x simpler (1 component vs 3 modules)
- ✅ No dependencies needed
- ✅ Works immediately
- ✅ Easy to remove later
- ✅ Perfect for debugging

**vs DevTools:**
- ✅ Always visible
- ✅ Works in production builds
- ✅ Shows backend logs too
- ✅ No F12 needed

---

## Implementation Time

**30 minutes:**
- 15 min: Create component
- 10 min: Add styles
- 5 min: Test

**vs Full Solution:** 8-10 hours

---

## Testing

```typescript
// Test it works (errors and warnings ONLY)
console.error('Test error');
console.warn('Test warning');
throw new Error('Test exception');

// Should see:
// 09:41:23.457 [ERR] Test error
// 09:41:23.458 [WARN] Test warning
// 09:41:23.459 [ERR] Test exception at Component.tsx:42

// Should NOT see regular logs:
console.log('This will not appear in debug console');
```

---

**Status:** Ready to implement
**Priority:** HIGH
**Time:** 30 minutes
**Dependencies:** None
