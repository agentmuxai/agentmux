#!/usr/bin/env node

/**
 * Simple Reactive Claude - No workspace needed, just reactive messaging demo
 *
 * Usage: node simple-reactive-claude.js <instance-name>
 * Example: node simple-reactive-claude.js Alice
 *
 * Features:
 * - Spawn Claude CLI with any name
 * - Watch for messages addressed to this instance
 * - Auto-inject messages into Claude's stdin
 * - Visual indication of wrapper vs Claude
 */

import { spawn } from 'child_process';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const colors = {
  reset: '\x1b[0m',
  wrapper: '\x1b[1;36m',    // Bright cyan
  message: '\x1b[1;35m',    // Bright magenta
  claude: '\x1b[0m',        // Normal
};

class SimpleReactiveClaude {
  constructor(instanceName) {
    this.instanceName = instanceName;
    this.process = null;
    this.messagesDir = path.join(os.homedir(), '.agentmux', 'shared', 'messages');
    this.processedMessages = new Set();
  }

  log(message, color = colors.wrapper) {
    console.log(`${color}[${this.instanceName}]${colors.reset} ${message}`);
  }

  async start() {
    // Ensure messages directory exists
    if (!fs.existsSync(this.messagesDir)) {
      fs.mkdirSync(this.messagesDir, { recursive: true });
    }

    this.log(`Starting Claude CLI instance...`);
    this.log(`Messages directory: ${this.messagesDir}`);

    // Add startup prompt explaining reactive messaging
    const startupPrompt = `You are a Claude instance named "${this.instanceName}".

You have access to the agentmux MCP tools to send messages to other Claude instances:
- mcp__agentmux__agentmux_send_message: Send a message to another instance
- mcp__agentmux__agentmux_list_messages: List recent messages

You can receive messages from other instances - they will appear as user input automatically.

Try it out! If another instance is running (e.g., "Bob"), you can message them:
mcp__agentmux__agentmux_send_message with to="Bob" and message="Hello Bob!"

For this demo, just respond naturally when you receive messages.`;

    // Spawn Claude CLI
    this.spawnClaude(startupPrompt);

    // Watch for messages
    this.watchMessages();

    this.log(`Ready! Instance "${this.instanceName}" is listening for messages.`);
    this.log(`To send a message TO this instance, address it to: "${this.instanceName}"`);
  }

  spawnClaude(startupPrompt) {
    // Spawn Claude CLI in current directory
    this.process = spawn('claude', [], {
      stdio: ['pipe', 'pipe', 'pipe'],
      cwd: process.cwd(),
      env: {
        ...process.env,
        AGENTMUX_INSTANCE_NAME: this.instanceName,
      }
    });

    // Send startup prompt
    setTimeout(() => {
      this.process.stdin.write(startupPrompt + '\n');
    }, 1000);

    // Capture stdout
    this.process.stdout.on('data', (data) => {
      // Just pass through Claude's output
      process.stdout.write(data);
    });

    // Capture stderr
    this.process.stderr.on('data', (data) => {
      this.log(`[ERROR] ${data.toString()}`, '\x1b[1;31m');
    });

    // Handle exit
    this.process.on('exit', (code) => {
      this.log(`Claude CLI exited with code ${code}`);
      process.exit(code);
    });

    // Allow user to type directly
    process.stdin.on('data', (data) => {
      this.process.stdin.write(data);
    });

    this.log(`✓ Claude spawned (PID: ${this.process.pid})`);
  }

  watchMessages() {
    // Get existing messages to skip
    try {
      const existing = fs.readdirSync(this.messagesDir)
        .filter(f => f.endsWith('.json'));
      existing.forEach(f => this.processedMessages.add(f));

      this.log(`Skipping ${existing.length} existing messages`);
    } catch (err) {
      this.log(`Error reading messages: ${err.message}`, '\x1b[1;31m');
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

      // Skip our own messages
      if (message.from.id === this.instanceName || message.from.name === this.instanceName) {
        this.processedMessages.add(filename);
        return;
      }

      // Display message
      this.log(`📨 Incoming message from ${message.from.name || message.from.id}`, colors.message);
      this.log(`   "${message.payload.text}"`, colors.message);

      // Inject into Claude's stdin
      const input = `\n[INCOMING MESSAGE from ${message.from.name || message.from.id}]: ${message.payload.text}\n\n`;
      this.process.stdin.write(input);

      this.processedMessages.add(filename);

    } catch (err) {
      this.log(`Error handling message: ${err.message}`, '\x1b[1;31m');
      this.processedMessages.add(filename);
    }
  }

  isMessageForMe(message) {
    const to = message.to;

    // Exact match
    if (to === this.instanceName) return true;

    // Broadcast
    if (to === '*') return true;

    // Wildcard pattern (e.g., "Alice-*")
    if (to.endsWith('*')) {
      const prefix = to.slice(0, -1);
      if (this.instanceName.startsWith(prefix)) return true;
    }

    return false;
  }

  stop() {
    this.log('Stopping...');
    if (this.process) {
      this.process.kill('SIGTERM');
    }
  }
}

// Handle graceful shutdown
function setupShutdownHandlers(wrapper) {
  const shutdown = () => {
    wrapper.stop();
    process.exit(0);
  };

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
}

// Main
async function main() {
  const instanceName = process.argv[2];

  if (!instanceName) {
    console.error('Usage: node simple-reactive-claude.js <instance-name>');
    console.error('Example: node simple-reactive-claude.js Alice');
    console.error('');
    console.error('Then in another terminal:');
    console.error('  node simple-reactive-claude.js Bob');
    console.error('');
    console.error('Tell Alice to message Bob using MCP tools!');
    process.exit(1);
  }

  const wrapper = new SimpleReactiveClaude(instanceName);
  setupShutdownHandlers(wrapper);

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

export { SimpleReactiveClaude };
