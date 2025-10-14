# AgentMux Desktop - Debug Console Specification

**Date:** 2025-10-13
**Author:** AgentX
**Status:** Design Complete - Ready for Implementation

---

## Problem Statement

Users cannot see frontend (JavaScript/SolidJS) or backend (Rust) errors when running the Desktop app. The Message Stream shows "Watcher: error" but provides no visibility into what's failing.

**Current Pain Points:**
- Frontend errors hidden (no browser console)
- Backend errors not visible (Rust logs not captured)
- Debugging requires running `tauri:dev` with terminal
- No persistent log history
- Cannot diagnose production issues

---

## Solution: Bottom-Mounted Debug Console

Add a **collapsible debug console** at the bottom of the window (below the status bar) that displays all application logs in real-time. Simple, temporary, convenient view for debugging.

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  App Window                                         │
│                                                      │
│  ┌────────────────────────────────────────────────┐ │
│  │  Header (AgentMux Title)                       │ │
│  └────────────────────────────────────────────────┘ │
│                                                      │
│  ┌────────────────────────────────────────────────┐ │
│  │  Main Content Area (Tabs: Dashboard, etc.)    │ │
│  │                                                 │ │
│  │                                                 │ │
│  └────────────────────────────────────────────────┘ │
│                                                      │
│  ┌────────────────────────────────────────────────┐ │
│  │  Status Bar (Connection info, etc.)            │ │
│  └────────────────────────────────────────────────┘ │
│                                                      │
│  ┌────────────────────────────────────────────────┐ │
│  │  Debug Console (Collapsible) ← NEW             │ │
│  │  ┌──────────────────────────────────────────┐  │ │
│  │  │ [▼ Debug Console] [Clear] [Copy]         │  │ │
│  │  ├──────────────────────────────────────────┤  │ │
│  │  │ 09:41:23.456 [FE] App starting...        │  │ │
│  │  │ 09:41:24.123 [BE] Bus started on :8765   │  │ │
│  │  │ 09:41:25.789 [ERR] Watcher: file not... │  │ │
│  │  │ ...                                       │  │ │
│  │  └──────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────┘

Components:
┌──────────────────────────────────────────────────┐
│  Frontend (SolidJS)                              │
│  ┌────────────────────────────────────────────┐  │
│  │  DebugConsole Component (bottom-mounted)   │  │
│  │  - Toggle show/hide                        │  │
│  │  - Display last 100 logs                   │  │
│  │  - Clear button                            │  │
│  │  - Copy all button                         │  │
│  │  - Auto-scroll to bottom                   │  │
│  └────────────────────────────────────────────┘  │
│           ↑                                       │
│           │ (Tauri events)                        │
│  ┌────────┴───────────────────────────────────┐  │
│  │  Console Interceptor (JavaScript)          │  │
│  │  - Intercepts console.log/error/warn       │  │
│  │  - Captures uncaught exceptions            │  │
│  │  - Sends to backend                        │  │
│  └────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
                    ↓ IPC
┌──────────────────────────────────────────────────┐
│  Backend (Rust/Tauri)                            │
│  ┌────────────────────────────────────────────┐  │
│  │  Simple Log Buffer                         │  │
│  │  - Stores last 100 logs                    │  │
│  │  - Emits "debug_log" events                │  │
│  └────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

---

## Data Model (Simplified)

### Debug Log Entry

```rust
// Simple structure - just timestamp, prefix, and message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugLog {
    pub timestamp: u64,    // Unix timestamp (ms)
    pub prefix: String,    // [FE], [BE], [ERR], [WARN]
    pub message: String,   // The log message
}
```

```typescript
// Frontend
interface DebugLog {
  timestamp: number;
  prefix: string;  // [FE], [BE], [ERR], [WARN]
  message: string;
}
```

---

## Implementation Plan

### Phase 1: Backend Log Manager

**New files:**
- `src-tauri/src/logger/mod.rs` - Module entry
- `src-tauri/src/logger/manager.rs` - LogManager implementation
- `src-tauri/src/logger/types.rs` - LogEntry, LogLevel, LogSource

**Dependencies (add to Cargo.toml):**
```toml
[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
chrono = "0.4"  # For timestamp formatting
```

**LogManager API:**
```rust
pub struct LogManager {
    logs: Arc<RwLock<VecDeque<LogEntry>>>,
    max_logs: usize,
    app_handle: tauri::AppHandle,
}

impl LogManager {
    pub fn new(max_logs: usize, app_handle: tauri::AppHandle) -> Self;

    pub fn add_log(&self, entry: LogEntry);

    pub fn get_recent_logs(&self, limit: Option<usize>, level: Option<LogLevel>, source: Option<LogSource>) -> Vec<LogEntry>;

    pub fn clear_logs(&self);

    pub fn export_logs(&self, path: &str) -> Result<(), String>;
}
```

**Tracing Integration:**
```rust
// In main.rs setup
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn setup_logging(log_manager: Arc<LogManager>) {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("info"))
        .with(CustomLogLayer::new(log_manager))
        .init();
}

// Custom layer that forwards to LogManager
struct CustomLogLayer {
    log_manager: Arc<LogManager>,
}

impl<S> Layer<S> for CustomLogLayer {
    // Implement layer trait to capture tracing events
}
```

---

### Phase 2: Tauri Commands

**Add to `src-tauri/src/main.rs`:**

```rust
#[tauri::command]
async fn log_message(
    state: State<'_, AppState>,
    level: String,
    source: String,
    message: String,
    context: Option<String>,
) -> Result<(), String> {
    let log_level = match level.as_str() {
        "trace" => LogLevel::Trace,
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    };

    let log_source = match source.as_str() {
        "frontend" => LogSource::Frontend,
        "backend" => LogSource::Backend,
        "bus" => LogSource::Bus,
        "agent" => LogSource::Agent,
        "system" => LogSource::System,
        _ => LogSource::Frontend,
    };

    let entry = LogEntry {
        id: format!("log-{}-{}", chrono::Utc::now().timestamp_millis(), rand::random::<u32>()),
        timestamp: chrono::Utc::now().timestamp_millis() as u64,
        level: log_level,
        source: log_source,
        message,
        context,
    };

    state.log_manager.add_log(entry);
    Ok(())
}

#[tauri::command]
async fn get_recent_logs(
    state: State<'_, AppState>,
    limit: Option<usize>,
    level: Option<String>,
    source: Option<String>,
) -> Result<Vec<LogEntry>, String> {
    let log_level = level.and_then(|l| match l.as_str() {
        "trace" => Some(LogLevel::Trace),
        "debug" => Some(LogLevel::Debug),
        "info" => Some(LogLevel::Info),
        "warn" => Some(LogLevel::Warn),
        "error" => Some(LogLevel::Error),
        _ => None,
    });

    let log_source = source.and_then(|s| match s.as_str() {
        "frontend" => Some(LogSource::Frontend),
        "backend" => Some(LogSource::Backend),
        "bus" => Some(LogSource::Bus),
        "agent" => Some(LogSource::Agent),
        "system" => Some(LogSource::System),
        _ => None,
    });

    Ok(state.log_manager.get_recent_logs(limit, log_level, log_source))
}

#[tauri::command]
async fn clear_logs(state: State<'_, AppState>) -> Result<(), String> {
    state.log_manager.clear_logs();
    Ok(())
}

#[tauri::command]
async fn export_logs(
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    state.log_manager.export_logs(&path)
}
```

**Register commands:**
```rust
tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
        // ... existing commands
        log_message,
        get_recent_logs,
        clear_logs,
        export_logs,
    ])
```

---

### Phase 3: Frontend Console Interceptor

**New file: `src/utils/logger.ts`**

```typescript
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type LogLevel = 'trace' | 'debug' | 'info' | 'warn' | 'error';
export type LogSource = 'frontend' | 'backend' | 'bus' | 'agent' | 'system';

export interface LogEntry {
  id: string;
  timestamp: number;
  level: LogLevel;
  source: LogSource;
  message: string;
  context?: string;
}

class Logger {
  private static instance: Logger;

  private constructor() {
    this.interceptConsole();
    this.setupErrorHandlers();
  }

  static getInstance(): Logger {
    if (!Logger.instance) {
      Logger.instance = new Logger();
    }
    return Logger.instance;
  }

  private interceptConsole() {
    const originalLog = console.log;
    const originalError = console.error;
    const originalWarn = console.warn;
    const originalDebug = console.debug;

    console.log = (...args: any[]) => {
      originalLog.apply(console, args);
      this.log('info', args.join(' '));
    };

    console.error = (...args: any[]) => {
      originalError.apply(console, args);
      this.log('error', args.join(' '));
    };

    console.warn = (...args: any[]) => {
      originalWarn.apply(console, args);
      this.log('warn', args.join(' '));
    };

    console.debug = (...args: any[]) => {
      originalDebug.apply(console, args);
      this.log('debug', args.join(' '));
    };
  }

  private setupErrorHandlers() {
    // Catch unhandled errors
    window.addEventListener('error', (event) => {
      this.log('error', `Uncaught error: ${event.message}`,
        `${event.filename}:${event.lineno}:${event.colno}`);
    });

    // Catch unhandled promise rejections
    window.addEventListener('unhandledrejection', (event) => {
      this.log('error', `Unhandled promise rejection: ${event.reason}`);
    });
  }

  async log(
    level: LogLevel,
    message: string,
    context?: string
  ): Promise<void> {
    try {
      await invoke('log_message', {
        level,
        source: 'frontend',
        message,
        context,
      });
    } catch (err) {
      // Fallback - don't create infinite loop
      console.error('[Logger] Failed to send log:', err);
    }
  }

  // Convenience methods
  trace(message: string, context?: string) {
    return this.log('trace', message, context);
  }

  debug(message: string, context?: string) {
    return this.log('debug', message, context);
  }

  info(message: string, context?: string) {
    return this.log('info', message, context);
  }

  warn(message: string, context?: string) {
    return this.log('warn', message, context);
  }

  error(message: string, context?: string) {
    return this.log('error', message, context);
  }
}

export const logger = Logger.getInstance();
```

**Initialize in `src/index.tsx`:**
```typescript
import { logger } from './utils/logger';

// Initialize logger immediately
logger.info('Application starting');
```

---

### Phase 4: LogViewer Component

**New file: `src/components/LogViewer.tsx`**

```typescript
import { createSignal, onMount, onCleanup, For, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { LogEntry, LogLevel, LogSource } from '../utils/logger';

export function LogViewer() {
  const [logs, setLogs] = createSignal<LogEntry[]>([]);
  const [filter, setFilter] = createSignal('');
  const [levelFilter, setLevelFilter] = createSignal<LogLevel | 'all'>('all');
  const [sourceFilter, setSourceFilter] = createSignal<LogSource | 'all'>('all');
  const [autoScroll, setAutoScroll] = createSignal(true);
  const [maxLogs, setMaxLogs] = createSignal(1000);

  let containerRef: HTMLDivElement | undefined;
  let pollInterval: number;

  onMount(async () => {
    // Initial load
    await refreshLogs();

    // Listen for new log events from backend
    await listen('log_entry', (event: any) => {
      const newLog = event.payload as LogEntry;
      setLogs([newLog, ...logs()].slice(0, maxLogs()));

      if (autoScroll() && containerRef) {
        containerRef.scrollTop = 0;
      }
    });

    // Poll for logs every 2 seconds (backup to events)
    pollInterval = setInterval(refreshLogs, 2000) as any;
  });

  onCleanup(() => {
    if (pollInterval) clearInterval(pollInterval);
  });

  async function refreshLogs() {
    try {
      const recentLogs = await invoke<LogEntry[]>('get_recent_logs', {
        limit: maxLogs(),
        level: levelFilter() === 'all' ? null : levelFilter(),
        source: sourceFilter() === 'all' ? null : sourceFilter(),
      });
      setLogs(recentLogs);
    } catch (err) {
      console.error('Failed to fetch logs:', err);
    }
  }

  async function handleClear() {
    if (confirm('Clear all logs?')) {
      try {
        await invoke('clear_logs');
        setLogs([]);
      } catch (err) {
        console.error('Failed to clear logs:', err);
      }
    }
  }

  async function handleExport() {
    // TODO: Use file picker dialog
    const filename = `agentmux-logs-${Date.now()}.txt`;
    try {
      await invoke('export_logs', { path: filename });
      alert(`Logs exported to ${filename}`);
    } catch (err) {
      console.error('Failed to export logs:', err);
      alert('Export failed: ' + err);
    }
  }

  function filteredLogs() {
    return logs().filter(log => {
      // Text filter
      if (filter() && !log.message.toLowerCase().includes(filter().toLowerCase())) {
        return false;
      }

      // Level filter
      if (levelFilter() !== 'all' && log.level !== levelFilter()) {
        return false;
      }

      // Source filter
      if (sourceFilter() !== 'all' && log.source !== sourceFilter()) {
        return false;
      }

      return true;
    });
  }

  function getLevelColor(level: LogLevel): string {
    switch (level) {
      case 'error': return '#ef5350';
      case 'warn': return '#ff9800';
      case 'info': return '#4a9eff';
      case 'debug': return '#999';
      case 'trace': return '#666';
      default: return '#e0e0e0';
    }
  }

  function getSourceBadge(source: LogSource): string {
    switch (source) {
      case 'frontend': return 'FE';
      case 'backend': return 'BE';
      case 'bus': return 'BUS';
      case 'agent': return 'AGT';
      case 'system': return 'SYS';
      default: return source.toUpperCase().slice(0, 3);
    }
  }

  function formatTimestamp(timestamp: number): string {
    const date = new Date(timestamp);
    return date.toLocaleTimeString('en-US', {
      hour12: false,
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      fractionalSecondDigits: 3,
    });
  }

  return (
    <div class="log-viewer">
      {/* Header */}
      <div class="log-header">
        <h2>Application Logs</h2>

        <div class="log-controls">
          {/* Text filter */}
          <input
            type="text"
            placeholder="Filter logs..."
            value={filter()}
            onInput={(e) => setFilter(e.currentTarget.value)}
            class="log-filter-input"
          />

          {/* Level filter */}
          <select
            value={levelFilter()}
            onChange={(e) => setLevelFilter(e.currentTarget.value as any)}
            class="log-filter-select"
          >
            <option value="all">All Levels</option>
            <option value="trace">Trace</option>
            <option value="debug">Debug</option>
            <option value="info">Info</option>
            <option value="warn">Warn</option>
            <option value="error">Error</option>
          </select>

          {/* Source filter */}
          <select
            value={sourceFilter()}
            onChange={(e) => setSourceFilter(e.currentTarget.value as any)}
            class="log-filter-select"
          >
            <option value="all">All Sources</option>
            <option value="frontend">Frontend</option>
            <option value="backend">Backend</option>
            <option value="bus">Bus</option>
            <option value="agent">Agent</option>
            <option value="system">System</option>
          </select>

          {/* Max logs */}
          <select
            value={maxLogs()}
            onChange={(e) => setMaxLogs(Number(e.currentTarget.value))}
            class="log-filter-select"
          >
            <option value="100">100 logs</option>
            <option value="500">500 logs</option>
            <option value="1000">1000 logs</option>
            <option value="5000">5000 logs</option>
          </select>

          {/* Auto-scroll toggle */}
          <label class="log-checkbox">
            <input
              type="checkbox"
              checked={autoScroll()}
              onChange={(e) => setAutoScroll(e.currentTarget.checked)}
            />
            Auto-scroll
          </label>

          {/* Action buttons */}
          <button onClick={handleClear} class="log-button log-button-clear">
            Clear
          </button>

          <button onClick={handleExport} class="log-button log-button-export">
            Export
          </button>

          <button onClick={refreshLogs} class="log-button log-button-refresh">
            Refresh
          </button>
        </div>
      </div>

      {/* Statistics */}
      <div class="log-stats">
        <span>Total: {logs().length}</span>
        <span>Filtered: {filteredLogs().length}</span>
        <span>Errors: {logs().filter(l => l.level === 'error').length}</span>
        <span>Warnings: {logs().filter(l => l.level === 'warn').length}</span>
      </div>

      {/* Log entries */}
      <div class="log-container" ref={containerRef}>
        <Show when={filteredLogs().length > 0} fallback={
          <div class="log-empty">No logs to display</div>
        }>
          <For each={filteredLogs()}>
            {(log) => (
              <div class="log-entry" data-level={log.level}>
                <span class="log-timestamp">{formatTimestamp(log.timestamp)}</span>

                <span
                  class="log-level"
                  style={{ color: getLevelColor(log.level) }}
                >
                  {log.level.toUpperCase()}
                </span>

                <span class="log-source-badge">{getSourceBadge(log.source)}</span>

                <span class="log-message">{log.message}</span>

                <Show when={log.context}>
                  <span class="log-context">{log.context}</span>
                </Show>
              </div>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
}
```

**Add styles to `src/styles.css`:**

```css
/* Log Viewer */
.log-viewer {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 1rem;
}

.log-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

.log-header h2 {
  margin: 0;
  font-size: 1.5rem;
}

.log-controls {
  display: flex;
  gap: 0.5rem;
  align-items: center;
}

.log-filter-input {
  padding: 0.5rem;
  border: 1px solid #3a3a3a;
  border-radius: 4px;
  background: #2a2a2a;
  color: #e0e0e0;
  width: 200px;
}

.log-filter-select {
  padding: 0.5rem;
  border: 1px solid #3a3a3a;
  border-radius: 4px;
  background: #2a2a2a;
  color: #e0e0e0;
}

.log-checkbox {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  color: #e0e0e0;
  cursor: pointer;
}

.log-button {
  padding: 0.5rem 1rem;
  border: 1px solid #3a3a3a;
  border-radius: 4px;
  background: #2a2a2a;
  color: #e0e0e0;
  cursor: pointer;
  transition: all 0.2s;
}

.log-button:hover {
  background: #3a3a3a;
}

.log-button-clear {
  color: #ef5350;
}

.log-button-export {
  color: #4a9eff;
}

.log-button-refresh {
  color: #66bb6a;
}

.log-stats {
  display: flex;
  gap: 1rem;
  padding: 0.5rem;
  background: #2a2a2a;
  border-radius: 4px;
  margin-bottom: 0.5rem;
  color: #999;
  font-size: 0.875rem;
}

.log-container {
  flex: 1;
  overflow-y: auto;
  background: #1a1a1a;
  border: 1px solid #3a3a3a;
  border-radius: 4px;
  padding: 0.5rem;
  font-family: 'Courier New', monospace;
  font-size: 0.875rem;
}

.log-entry {
  display: flex;
  gap: 0.5rem;
  padding: 0.25rem;
  border-bottom: 1px solid #2a2a2a;
  color: #e0e0e0;
}

.log-entry:hover {
  background: #2a2a2a;
}

.log-timestamp {
  color: #666;
  font-weight: bold;
  min-width: 100px;
}

.log-level {
  font-weight: bold;
  min-width: 60px;
}

.log-source-badge {
  display: inline-block;
  padding: 0.125rem 0.375rem;
  background: #3a3a3a;
  border-radius: 3px;
  font-size: 0.75rem;
  font-weight: bold;
  color: #999;
  min-width: 40px;
  text-align: center;
}

.log-message {
  flex: 1;
  word-break: break-word;
}

.log-context {
  color: #666;
  font-style: italic;
  margin-left: auto;
}

.log-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #666;
  font-size: 1rem;
}
```

---

### Phase 5: Add Logs Tab to App

**Update `src/App.tsx`:**

```typescript
import { LogViewer } from './components/LogViewer';

// Add to tab list
const tabs = [
  { id: 'dashboard', label: 'Dashboard' },
  { id: 'bus', label: 'Bus' },
  { id: 'agents', label: 'Agents' },
  { id: 'messages', label: 'Messages' },
  { id: 'logs', label: 'Logs' },  // ← NEW
];

// Add to render
<Show when={activeTab() === 'logs'}>
  <LogViewer />
</Show>
```

---

## Features

### Core Features

1. **Real-time Log Display**
   - Frontend logs (console.log, errors, warnings)
   - Backend logs (Rust tracing)
   - Bus logs (WebSocket events)
   - Agent logs (agent-related events)

2. **Filtering**
   - Text search across all messages
   - Filter by log level (trace/debug/info/warn/error)
   - Filter by source (frontend/backend/bus/agent/system)
   - Configurable history size (100/500/1000/5000)

3. **Controls**
   - Auto-scroll toggle
   - Clear all logs
   - Export to file
   - Manual refresh

4. **Statistics**
   - Total log count
   - Filtered log count
   - Error count
   - Warning count

5. **Visual Design**
   - Color-coded log levels
   - Source badges (FE, BE, BUS, AGT, SYS)
   - Timestamps with milliseconds
   - Monospace font for readability
   - Context/metadata display

---

## Performance Considerations

### Memory Management

**Circular Buffer:**
- Max 1000-5000 logs in memory
- Oldest logs automatically removed
- User-configurable limit

**Event Throttling:**
- Poll every 2 seconds (configurable)
- Event-based updates for immediate feedback
- Auto-scroll only when enabled

### Optimization

**Backend:**
- RwLock for thread-safe reads
- VecDeque for efficient circular buffer
- Async logging to prevent blocking

**Frontend:**
- SolidJS fine-grained reactivity
- Virtual scrolling for large log lists (future enhancement)
- Filtered logs computed on demand

---

## Testing Strategy

### Unit Tests

**Backend (Rust):**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_log_manager_add_log() { ... }

    #[test]
    fn test_log_manager_circular_buffer() { ... }

    #[test]
    fn test_log_filtering() { ... }

    #[test]
    fn test_log_export() { ... }
}
```

**Frontend (Vitest):**
```typescript
describe('LogViewer', () => {
  it('displays logs correctly', () => { ... });

  it('filters logs by level', () => { ... });

  it('filters logs by source', () => { ... });

  it('searches log messages', () => { ... });

  it('clears logs', () => { ... });
});
```

### Integration Tests

1. Log a message from frontend → Verify appears in backend
2. Log from Rust → Verify appears in frontend
3. Filter logs → Verify correct logs shown
4. Clear logs → Verify logs removed
5. Export logs → Verify file created with correct content

---

## Future Enhancements

### High Priority

1. **Persistent Storage**
   - Save logs to SQLite database
   - Load historical logs on startup
   - Log rotation (max file size)

2. **File Picker Dialog**
   - Use Tauri file dialog for export
   - Allow user to choose export location
   - Support multiple export formats (JSON, CSV, TXT)

3. **Virtual Scrolling**
   - Handle 10,000+ logs efficiently
   - Only render visible entries
   - Smooth scrolling performance

### Medium Priority

4. **Log Streaming**
   - WebSocket streaming instead of polling
   - Instant log updates
   - Lower latency

5. **Advanced Filtering**
   - Regex support
   - Date range filtering
   - Multiple simultaneous filters
   - Save filter presets

6. **Log Details Modal**
   - Click log entry for full details
   - Stack trace display for errors
   - Copy log to clipboard

### Low Priority

7. **Log Analysis**
   - Error rate charts
   - Log volume over time
   - Pattern detection

8. **Log Alerts**
   - Desktop notifications for errors
   - Sound alerts for critical logs
   - Email notifications (optional)

9. **Multi-file Export**
   - Export to multiple formats
   - Scheduled exports
   - Cloud upload (S3, etc.)

---

## Error Handling

### Backend Errors

```rust
// If logging fails, don't crash the app
if let Err(e) = log_manager.add_log(entry) {
    eprintln!("Failed to add log: {}", e);
    // Continue execution
}
```

### Frontend Errors

```typescript
try {
  await invoke('log_message', { ... });
} catch (err) {
  // Fallback - don't create infinite loop
  console.error('[Logger] Failed to send log:', err);
}
```

### Circular Dependency Prevention

- Logger must not log its own failures
- Use separate error tracking for logger issues
- Fallback to console.error for logger errors

---

## Security Considerations

### Log Content Sanitization

**Do NOT log:**
- Passwords or API keys
- Authentication tokens
- Personal identifiable information (PII)
- Credit card numbers
- Session cookies

**Safe to log:**
- User IDs (anonymized)
- Error messages (sanitized)
- System metrics
- Application events
- Debug information (non-sensitive)

### Export Security

- Warn user before exporting logs
- Sanitize exported logs (remove sensitive data)
- Require user confirmation for export
- Store exports in secure location

---

## Dependencies

### New Dependencies

**Rust (Cargo.toml):**
```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
chrono = "0.4"
```

**Frontend:**
No new dependencies required (uses existing Tauri APIs)

---

## Migration Path

### For Existing Installations

1. **Backend changes are transparent** - Logs automatically captured
2. **Frontend changes are additive** - New tab added, existing tabs unchanged
3. **No breaking changes** - All existing functionality preserved
4. **Graceful degradation** - If logging fails, app continues to work

---

## Success Criteria

✅ **Functional:**
- All frontend errors visible in Logs tab
- All backend errors visible in Logs tab
- Filtering and search work correctly
- Export creates valid log file

✅ **Performance:**
- No noticeable impact on app performance
- Logs appear in < 100ms
- Can handle 1000+ logs without lag

✅ **Usability:**
- Easy to find specific errors
- Clear visual hierarchy
- Intuitive controls

✅ **Testing:**
- 95%+ code coverage
- All integration tests passing
- No memory leaks

---

## Implementation Timeline

### Phase 1: Backend (2-3 hours)
- Create LogManager module
- Add Tauri commands
- Integrate tracing crate

### Phase 2: Frontend Logger (1 hour)
- Create logger utility
- Intercept console
- Setup error handlers

### Phase 3: LogViewer Component (2-3 hours)
- Build UI component
- Add filtering/search
- Implement controls

### Phase 4: Integration (1 hour)
- Add Logs tab to App
- Test end-to-end
- Fix any bugs

### Phase 5: Testing (2 hours)
- Write unit tests
- Write integration tests
- Verify all features

**Total Estimated Time: 8-10 hours**

---

## Acceptance Criteria

- [ ] Logs tab visible in Desktop app
- [ ] Frontend console.log appears in Logs
- [ ] Frontend errors appear in Logs
- [ ] Backend Rust logs appear in Logs
- [ ] Bus events appear in Logs
- [ ] Text search works correctly
- [ ] Level filter works correctly
- [ ] Source filter works correctly
- [ ] Auto-scroll toggle works
- [ ] Clear logs button works
- [ ] Export logs creates valid file
- [ ] Statistics display correctly
- [ ] Color coding visible
- [ ] Timestamps formatted correctly
- [ ] No performance degradation
- [ ] Tests passing

---

**Status:** Ready for Implementation
**Priority:** HIGH (critical for debugging)
**Blocking Issues:** None
**Dependencies:** None (uses existing infrastructure)

---

**Document:** SPEC_LOG_VIEWER.md
**Version:** 1.0
**Last Updated:** 2025-10-13
