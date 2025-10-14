import { createSignal, For, Show, onMount, onCleanup } from 'solid-js';
import { listen } from '@tauri-apps/api/event';

interface DebugLog {
  time: string;
  prefix: string;
  message: string;
  expanded?: boolean;
}

export function DebugConsole() {
  const [logs, setLogs] = createSignal<DebugLog[]>([]);
  const [collapsed, setCollapsed] = createSignal(false);
  const [height, setHeight] = createSignal(250); // Default height in pixels
  let resizing = false;

  onMount(() => {
    // Intercept ALL console methods for debugging
    const originalLog = console.log;
    const originalError = console.error;
    const originalWarn = console.warn;

    // Helper to serialize arguments (handles objects, arrays, etc.)
    const serializeArgs = (...args: any[]): string => {
      return args.map(arg => {
        if (typeof arg === 'object' && arg !== null) {
          try {
            return JSON.stringify(arg, null, 2);
          } catch (e) {
            return String(arg);
          }
        }
        return String(arg);
      }).join(' ');
    };

    console.log = (...args: any[]) => {
      originalLog.apply(console, args);
      addLog('[LOG]', serializeArgs(...args));
    };

    console.error = (...args: any[]) => {
      originalError.apply(console, args);
      addLog('[ERR]', serializeArgs(...args));
    };

    console.warn = (...args: any[]) => {
      originalWarn.apply(console, args);
      addLog('[WARN]', serializeArgs(...args));
    };

    // Catch uncaught errors
    window.addEventListener('error', (event) => {
      addLog('[ERR]', `${event.message} at ${event.filename}:${event.lineno}`);
    });

    // Catch unhandled promise rejections
    window.addEventListener('unhandledrejection', (event) => {
      addLog('[ERR]', `Unhandled promise rejection: ${event.reason}`);
    });

    // Listen for backend logs
    listen('debug_log', (event) => {
      const message = event.payload as string;
      addLog('[RUST]', message);
    });

    // Add resize functionality
    const handleMouseMove = (e: MouseEvent) => {
      if (resizing) {
        const newHeight = window.innerHeight - e.clientY;
        setHeight(Math.max(100, Math.min(600, newHeight))); // Min 100px, max 600px
      }
    };

    const handleMouseUp = () => {
      if (resizing) {
        resizing = false;
        document.body.style.cursor = 'default';
        document.body.style.userSelect = '';
      }
    };

    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);

    onCleanup(() => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    });
  });

  function handleResizeStart(e: MouseEvent) {
    e.preventDefault();
    resizing = true;
    document.body.style.cursor = 'ns-resize';
    document.body.style.userSelect = 'none';
  }

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
    <div class="debug-console" style={{ height: collapsed() ? 'auto' : `${height()}px` }}>
      {/* Resize handle */}
      <Show when={!collapsed()}>
        <div
          class="debug-console-resize-handle"
          onMouseDown={handleResizeStart}
          title="Drag to resize"
        >
          <div class="debug-console-resize-indicator">⋮</div>
        </div>
      </Show>

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
                      'debug-prefix-rust': log.prefix === '[RUST]',
                    }}
                  >
                    {log.prefix}
                  </span>
                  <pre class="debug-message">{log.message}</pre>
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
