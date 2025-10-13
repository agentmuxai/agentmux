#!/usr/bin/env node

/**
 * PTY Claude Wrapper - Keeps Claude CLI inside Desktop app
 *
 * This wrapper uses node-pty to spawn Claude in a pseudoterminal,
 * allowing full interactivity while keeping everything inside the Desktop app.
 *
 * Features:
 * - Full PTY support (colors, interactive prompts, etc.)
 * - Watches for messages and injects them
 * - Streams output via WebSocket to Desktop UI
 * - Allows bidirectional communication
 */

import pty from 'node-pty';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { WebSocketServer } from 'ws';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

class PTYClaudeWrapper {
  constructor(instanceName, wsPort) {
    this.instanceName = instanceName;
    this.wsPort = wsPort;
    this.ptyProcess = null;
    this.wss = null;
    this.clients = new Set();
    this.messagesDir = path.join(os.homedir(), '.agentmux', 'shared', 'messages');
    this.processedMessages = new Set();
    this.outputBuffer = '';
  }

  async start() {
    // Ensure messages directory exists
    if (!fs.existsSync(this.messagesDir)) {
      fs.mkdirSync(this.messagesDir, { recursive: true });
    }

    // Start WebSocket server for UI communication
    await this.startWebSocketServer();

    // Spawn Claude in PTY
    this.spawnClaude();

    // Watch for messages
    this.watchMessages();

    this.broadcast({
      type: 'status',
      data: {
        instanceName: this.instanceName,
        status: 'ready',
        wsPort: this.wsPort,
      }
    });
  }

  async startWebSocketServer() {
    this.wss = new WebSocketServer({ port: this.wsPort });

    this.wss.on('connection', (ws) => {
      console.log(`[${this.instanceName}] UI connected`);
      this.clients.add(ws);

      // Send current output buffer to new client
      if (this.outputBuffer) {
        ws.send(JSON.stringify({
          type: 'output',
          data: this.outputBuffer,
        }));
      }

      // Handle input from UI
      ws.on('message', (data) => {
        try {
          const message = JSON.parse(data.toString());

          if (message.type === 'input' && this.ptyProcess) {
            // Write user input to PTY
            this.ptyProcess.write(message.data);
          }
        } catch (err) {
          console.error(`[${this.instanceName}] Error handling WS message:`, err);
        }
      });

      ws.on('close', () => {
        console.log(`[${this.instanceName}] UI disconnected`);
        this.clients.delete(ws);
      });
    });

    console.log(`[${this.instanceName}] WebSocket server listening on ws://localhost:${this.wsPort}`);
  }

  spawnClaude() {
    const shell = process.platform === 'win32' ? 'powershell.exe' : 'bash';

    // Spawn Claude CLI in PTY
    this.ptyProcess = pty.spawn(shell, [], {
      name: 'xterm-256color',
      cols: 120,
      rows: 30,
      cwd: process.cwd(),
      env: {
        ...process.env,
        AGENTMUX_INSTANCE_NAME: this.instanceName,
        TERM: 'xterm-256color',
      }
    });

    // Send initial command to start Claude
    setTimeout(() => {
      this.ptyProcess.write('claude\r');

      // Send startup context
      setTimeout(() => {
        const startupPrompt = `You are a Claude instance named "${this.instanceName}".

You have access to agentmux MCP tools to communicate with other instances:
- mcp__agentmux__agentmux_send_message: Send a message
- mcp__agentmux__agentmux_list_messages: List messages

You can receive messages from other instances - they will appear automatically.

Ready to collaborate!\r`;

        this.ptyProcess.write(startupPrompt);
      }, 2000);
    }, 1000);

    // Handle output
    this.ptyProcess.onData((data) => {
      this.outputBuffer += data;

      // Keep buffer reasonable size (last 100KB)
      if (this.outputBuffer.length > 100000) {
        this.outputBuffer = this.outputBuffer.slice(-50000);
      }

      // Broadcast to all connected UIs
      this.broadcast({
        type: 'output',
        data: data,
      });
    });

    // Handle exit
    this.ptyProcess.onExit(({ exitCode, signal }) => {
      console.log(`[${this.instanceName}] Claude exited (code: ${exitCode}, signal: ${signal})`);

      this.broadcast({
        type: 'status',
        data: {
          instanceName: this.instanceName,
          status: 'exited',
          exitCode,
          signal,
        }
      });

      // Clean up
      this.stop();
    });

    console.log(`[${this.instanceName}] Claude spawned in PTY (PID: ${this.ptyProcess.pid})`);
  }

  watchMessages() {
    // Get existing messages to skip
    try {
      const existing = fs.readdirSync(this.messagesDir)
        .filter(f => f.endsWith('.json'));
      existing.forEach(f => this.processedMessages.add(f));

      console.log(`[${this.instanceName}] Skipping ${existing.length} existing messages`);
    } catch (err) {
      console.error(`[${this.instanceName}] Error reading messages:`, err);
    }

    // Watch for new messages
    fs.watch(this.messagesDir, (eventType, filename) => {
      if (!filename?.endsWith('.json')) return;
      if (this.processedMessages.has(filename)) return;

      setTimeout(() => {
        this.handleNewMessage(filename);
      }, 100);
    });
  }

  handleNewMessage(filename) {
    if (this.processedMessages.has(filename)) return;

    const filePath = path.join(this.messagesDir, filename);

    try {
      if (!fs.existsSync(filePath)) return;

      const content = fs.readFileSync(filePath, 'utf-8');
      const message = JSON.parse(content);

      // Check if message is for this instance
      if (!this.isMessageForMe(message)) {
        this.processedMessages.add(filename);
        return;
      }

      // Skip own messages
      if (message.from.id === this.instanceName || message.from.name === this.instanceName) {
        this.processedMessages.add(filename);
        return;
      }

      console.log(`[${this.instanceName}] 📨 Incoming message from ${message.from.name || message.from.id}`);

      // Broadcast message notification to UI
      this.broadcast({
        type: 'message',
        data: {
          from: message.from.name || message.from.id,
          text: message.payload.text,
          timestamp: message.timestamp,
        }
      });

      // Inject into Claude's PTY
      const input = `\n[INCOMING MESSAGE from ${message.from.name || message.from.id}]: ${message.payload.text}\n\n`;
      if (this.ptyProcess) {
        this.ptyProcess.write(input);
      }

      this.processedMessages.add(filename);

    } catch (err) {
      console.error(`[${this.instanceName}] Error handling message:`, err);
      this.processedMessages.add(filename);
    }
  }

  isMessageForMe(message) {
    const to = message.to;

    // Exact match
    if (to === this.instanceName) return true;

    // Broadcast
    if (to === '*') return true;

    // Wildcard pattern
    if (to.endsWith('*')) {
      const prefix = to.slice(0, -1);
      if (this.instanceName.startsWith(prefix)) return true;
    }

    return false;
  }

  broadcast(message) {
    const data = JSON.stringify(message);

    this.clients.forEach((client) => {
      if (client.readyState === 1) { // WebSocket.OPEN
        client.send(data);
      }
    });
  }

  resize(cols, rows) {
    if (this.ptyProcess) {
      this.ptyProcess.resize(cols, rows);
    }
  }

  stop() {
    console.log(`[${this.instanceName}] Stopping...`);

    if (this.ptyProcess) {
      this.ptyProcess.kill();
      this.ptyProcess = null;
    }

    if (this.wss) {
      this.wss.close();
      this.wss = null;
    }

    this.clients.clear();
  }
}

// Main
async function main() {
  const instanceName = process.argv[2];
  const wsPort = parseInt(process.argv[3]) || (9000 + Math.floor(Math.random() * 1000));

  if (!instanceName) {
    console.error('Usage: node pty-claude-wrapper.js <instance-name> [ws-port]');
    console.error('Example: node pty-claude-wrapper.js Alice 9000');
    process.exit(1);
  }

  const wrapper = new PTYClaudeWrapper(instanceName, wsPort);

  // Handle graceful shutdown
  process.on('SIGINT', () => {
    wrapper.stop();
    process.exit(0);
  });

  process.on('SIGTERM', () => {
    wrapper.stop();
    process.exit(0);
  });

  try {
    await wrapper.start();
  } catch (err) {
    console.error('Failed to start wrapper:', err);
    process.exit(1);
  }
}

// Run if called directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}

export { PTYClaudeWrapper };
