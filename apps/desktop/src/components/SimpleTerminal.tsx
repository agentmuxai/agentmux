import { Component, onMount, onCleanup, createSignal } from 'solid-js';
import AnsiToHtml from 'ansi-to-html';

interface SimpleTerminalProps {
  instanceName: string;
  wsPort: number;
}

const SimpleTerminal: Component<SimpleTerminalProps> = (props) => {
  const [output, setOutput] = createSignal('');
  const [input, setInput] = createSignal('');
  const [isConnected, setIsConnected] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let ws: WebSocket | null = null;
  let outputRef: HTMLDivElement | undefined;

  onMount(() => {
    connectWebSocket();
  });

  const connectWebSocket = () => {
    const wsUrl = `ws://localhost:${props.wsPort}`;
    console.log(`[${props.instanceName}] [WS] Attempting to connect to ${wsUrl}...`);

    try {
      ws = new WebSocket(wsUrl);
      console.log(`[${props.instanceName}] [WS] WebSocket object created, readyState: ${ws.readyState} (CONNECTING)`);

      ws.onopen = () => {
        console.log(`[${props.instanceName}] [WS] ✓ Connection opened successfully, readyState: ${ws?.readyState} (OPEN)`);
        setIsConnected(true);
        setError(null);
        setOutput(prev => prev + `\x1b[1;32m[Connected to ${props.instanceName}]\x1b[0m\n`);
      };

      ws.onmessage = (event) => {
        const text = event.data;
        console.log(`[${props.instanceName}] [WS] ← Received message (${text.length} bytes)`);
        setOutput(prev => prev + text);

        // Auto-scroll to bottom
        if (outputRef) {
          outputRef.scrollTop = outputRef.scrollHeight;
        }
      };

      ws.onerror = (err) => {
        console.error(`[${props.instanceName}] [WS] ✗ Error occurred, readyState: ${ws?.readyState}`, err);
        console.error(`[${props.instanceName}] [WS] Error details:`, JSON.stringify(err, null, 2));
        setError('Connection error');
        setIsConnected(false);
      };

      ws.onclose = (event) => {
        console.log(`[${props.instanceName}] [WS] ✗ Connection closed`);
        console.log(`[${props.instanceName}] [WS] Close code: ${event.code}`);
        console.log(`[${props.instanceName}] [WS] Close reason: "${event.reason}"`);
        console.log(`[${props.instanceName}] [WS] Was clean: ${event.wasClean}`);
        console.log(`[${props.instanceName}] [WS] readyState: ${ws?.readyState} (CLOSED)`);

        setIsConnected(false);

        // Attempt reconnect after 2 seconds
        console.log(`[${props.instanceName}] [WS] Scheduling reconnect in 2 seconds...`);
        setTimeout(() => {
          console.log(`[${props.instanceName}] [WS] Reconnect timer fired, attempting reconnect...`);
          connectWebSocket();
        }, 2000);
      };
    } catch (err: any) {
      console.error(`[${props.instanceName}] [WS] ✗ Failed to create WebSocket:`, err);
      console.error(`[${props.instanceName}] [WS] Exception details:`, err.message, err.stack);
      setError(err.message);
    }
  };

  const handleSendInput = () => {
    const text = input();
    if (!text || !ws || ws.readyState !== WebSocket.OPEN) {
      console.warn(`[${props.instanceName}] Cannot send: ${!text ? 'empty input' : 'WebSocket not ready'}`);
      return;
    }

    try {
      console.log(`[${props.instanceName}] [WS] → Sending input: "${text}" (${text.length} chars)`);
      ws.send(text + '\n');
      console.log(`[${props.instanceName}] [WS] ✓ Input sent successfully`);
      setInput('');
      setError(null);
    } catch (err: any) {
      console.error(`[${props.instanceName}] [WS] ✗ Failed to send input:`, err);
      setError(err.toString());
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();

      // If input is empty, send bare Enter (CR) for menu confirmation
      if (!input() || input().trim() === '') {
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send('\r');  // Carriage return for menu selection
          console.log(`[${props.instanceName}] [WS] → Sent Enter key (CR)`);
        }
      } else {
        handleSendInput();
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      setInput('');  // Clear input on Escape
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      e.stopPropagation();
      // Send up arrow control sequence
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('\x1b[A');  // ANSI escape code for up arrow
        console.log(`[${props.instanceName}] [WS] → Sent up arrow key`);
      }
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      e.stopPropagation();
      // Send down arrow control sequence
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('\x1b[B');  // ANSI escape code for down arrow
        console.log(`[${props.instanceName}] [WS] → Sent down arrow key`);
      }
    }
  };

  onCleanup(() => {
    if (ws) {
      ws.close();
    }
  });

  return (
    <div class="simple-terminal">
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
        ref={outputRef}
        class="terminal-output"
        innerHTML={formatOutput(output())}
      />

      <div class="terminal-input-area">
        <input
          type="text"
          class="terminal-input"
          placeholder="Enter to confirm, ↑↓ to navigate, Esc to clear"
          value={input()}
          onInput={(e) => setInput(e.currentTarget.value)}
          onKeyDown={handleKeyDown}
          disabled={!isConnected()}
        />
      </div>
    </div>
  );
};

// ANSI color code converter (using ansi-to-html library)
const ansiConverter = new AnsiToHtml({
  fg: '#e0e0e0',      // Default foreground color
  bg: '#0a0a0a',      // Default background color
  newline: true,      // Convert \n to <br/>
  escapeXML: true,    // Escape HTML entities
  stream: false,      // Don't maintain state between calls
});

function formatOutput(text: string): string {
  return ansiConverter.toHtml(text);
}

export default SimpleTerminal;
