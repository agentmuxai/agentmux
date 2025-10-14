import { Component, onMount, onCleanup, createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';

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

  const handleSendInput = async () => {
    const text = input();
    if (!text) return;

    try {
      await invoke('send_claude_input', {
        instanceName: props.instanceName,
        input: text + '\n',
      });
      setInput('');
    } catch (err: any) {
      console.error(`[${props.instanceName}] Failed to send input:`, err);
      setError(err.toString());
    }
  };

  const handleKeyPress = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSendInput();
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
          placeholder="Type your input and press Enter..."
          value={input()}
          onInput={(e) => setInput(e.currentTarget.value)}
          onKeyPress={handleKeyPress}
          disabled={!isConnected()}
        />
        <button
          class="terminal-send"
          onClick={handleSendInput}
          disabled={!isConnected() || !input()}
        >
          Send
        </button>
      </div>
    </div>
  );
};

// Simple ANSI color code parser
function formatOutput(text: string): string {
  // Convert ANSI codes to HTML
  return text
    .replace(/\x1b\[1;32m/g, '<span style="color: #4ade80; font-weight: bold;">')
    .replace(/\x1b\[1;31m/g, '<span style="color: #ef4444; font-weight: bold;">')
    .replace(/\x1b\[1;33m/g, '<span style="color: #fbbf24; font-weight: bold;">')
    .replace(/\x1b\[1;34m/g, '<span style="color: #60a5fa; font-weight: bold;">')
    .replace(/\x1b\[1;35m/g, '<span style="color: #c084fc; font-weight: bold;">')
    .replace(/\x1b\[1;36m/g, '<span style="color: #22d3ee; font-weight: bold;">')
    .replace(/\x1b\[0m/g, '</span>')
    .replace(/\n/g, '<br/>');
}

export default SimpleTerminal;
