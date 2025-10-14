import { createSignal, For, Show, onMount } from 'solid-js';
import { listen } from '@tauri-apps/api/event';

interface DebugLog {
  time: string;
  prefix: string;
  message: string;
}

export function DebugConsole() {
  const [logs, setLogs] = createSignal<DebugLog[]>([]);
  const [collapsed, setCollapsed] = createSignal(false);

  onMount(() => {
    // Intercept ALL console methods for debugging
    const originalLog = console.log;
    const originalError = console.error;
    const originalWarn = console.warn;

    console.log = (...args: any[]) => {
      originalLog.apply(console, args);
      addLog('[LOG]', args.join(' '));
    };

    console.error = (...args: any[]) => {
      originalError.apply(console, args);
      addLog('[ERR]', args.join(' '));
    };

    console.warn = (...args: any[]) => {
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

    // Listen for backend logs
    listen('debug_log', (event) => {
      const message = event.payload as string;
      addLog('[RUST]', message);
    });
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
                      'debug-prefix-rust': log.prefix === '[RUST]',
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
