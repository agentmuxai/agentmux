import { Component, onMount, onCleanup, createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';

interface EmbeddedTerminalProps {
  instanceName: string;
  wsPort: number;
  onChangeDirectory?: (instanceName: string, newPath: string) => void;
}

const EmbeddedTerminal: Component<EmbeddedTerminalProps> = (props) => {
  let terminalRef: HTMLDivElement | undefined;
  let terminal: Terminal | null = null;
  let fitAddon: FitAddon | null = null;
  let ws: WebSocket | null = null;
  const [isConnected, setIsConnected] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // Safety check: Don't render if props are invalid
  if (!props.instanceName || !props.wsPort) {
    console.error('[EmbeddedTerminal] Invalid props:', { instanceName: props.instanceName, wsPort: props.wsPort });
    return (
      <div class="embedded-terminal">
        <div class="pane-error">
          <p>⚠️ Invalid terminal configuration</p>
          <p class="error-detail">Missing instanceName or wsPort</p>
        </div>
      </div>
    );
  }

  onMount(() => {
    if (!terminalRef) return;

    // Create terminal
    terminal = new Terminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: '"Cascadia Code", "Fira Code", "Courier New", monospace',
      theme: {
        background: '#1a1a1a',
        foreground: '#e0e0e0',
        cursor: '#4a9eff',
        selectionBackground: '#3a3a3a',
      },
      rows: 30,
      cols: 120,
    });

    // Add addons
    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(new WebLinksAddon());

    // Open terminal in DOM
    terminal.open(terminalRef);
    fitAddon.fit();

    // Handle user input
    terminal.onData(async (data) => {
      // Send input directly via Tauri command for better reliability
      try {
        await invoke('send_claude_input', {
          instanceName: props.instanceName,
          input: data,
        });
      } catch (err) {
        console.error(`[${props.instanceName}] Failed to send input:`, err);
      }
    });

    // Connect to WebSocket
    connectWebSocket();

    // Handle window resize
    const handleResize = () => {
      if (fitAddon) {
        fitAddon.fit();
      }
    };

    window.addEventListener('resize', handleResize);

    onCleanup(() => {
      window.removeEventListener('resize', handleResize);

      if (ws) {
        ws.close();
      }

      if (terminal) {
        terminal.dispose();
      }
    });
  });

  const connectWebSocket = () => {
    // Defensive check: don't connect if wsPort is invalid
    if (!props.wsPort || props.wsPort <= 0) {
      console.warn(`[${props.instanceName}] Invalid wsPort: ${props.wsPort} - skipping WebSocket connection`);
      setError(`Invalid port: ${props.wsPort}`);
      return;
    }

    const wsUrl = `ws://localhost:${props.wsPort}`;

    try {
      ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        console.log(`[${props.instanceName}] Connected to PTY wrapper`);
        setIsConnected(true);
        setError(null);

        if (terminal) {
          terminal.writeln(`\x1b[1;32m[Connected to ${props.instanceName}]\x1b[0m`);
        }
      };

      ws.onmessage = (event) => {
        // Backend sends raw PTY output as plain text (not JSON)
        // xterm.js handles all ANSI sequences internally
        if (terminal) {
          terminal.write(event.data);
        }
      };

      ws.onerror = (err) => {
        console.error(`[${props.instanceName}] WebSocket error:`, err);
        setError('Connection error');
        setIsConnected(false);
      };

      ws.onclose = () => {
        console.log(`[${props.instanceName}] WebSocket closed`);
        setIsConnected(false);

        // Attempt reconnect after 2 seconds
        setTimeout(() => {
          if (terminal) {
            connectWebSocket();
          }
        }, 2000);
      };
    } catch (err: any) {
      console.error(`[${props.instanceName}] Failed to create WebSocket:`, err);
      setError(err.message);
    }
  };

  const handleContextMenu = async (e: MouseEvent) => {
    e.preventDefault();

    if (!props.onChangeDirectory) return;

    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: 'Select Working Directory',
      });

      if (selected && typeof selected === 'string') {
        console.log(`[${props.instanceName}] Changing directory to: ${selected}`);
        props.onChangeDirectory(props.instanceName, selected);
      }
    } catch (err) {
      console.error(`[${props.instanceName}] Failed to select directory:`, err);
    }
  };

  return (
    <div class="embedded-terminal">
      <div class="terminal-header">
        <span class="terminal-title">
          <span class={`status-dot ${isConnected() ? 'online' : 'offline'}`}></span>
          {props.instanceName}
        </span>
        {error() && (
          <span class="terminal-error">{error()}</span>
        )}
        <span class="terminal-port">ws://localhost:{props.wsPort}</span>
      </div>
      <div
        ref={terminalRef}
        class="terminal-container"
        onContextMenu={handleContextMenu}
      ></div>
    </div>
  );
};

export default EmbeddedTerminal;
