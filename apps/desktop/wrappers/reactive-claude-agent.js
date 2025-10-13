#!/usr/bin/env node

/**
 * Reactive Claude CLI Agent
 * Spawns Claude CLI and integrates with AgentMux reactive messaging
 */

import { spawn } from 'child_process';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

class ReactiveCLIAgent {
  constructor(agentId, cliCommand = 'claude') {
    this.agentId = agentId;
    this.cliCommand = cliCommand;
    this.process = null;
    this.messagesDir = path.join(os.homedir(), '.agentmux', 'shared', 'messages');
    this.agentDir = path.join(os.homedir(), '.agentmux', 'desktop', 'agents', agentId);
    this.fullOutput = '';
    this.processedMessages = new Set();
    this.messageQueue = [];
    this.isProcessing = false;
    this.lastStatusWrite = 0;

    // Output buffer size limit (1MB max)
    this.MAX_OUTPUT_SIZE = 1024 * 1024;
  }

  async start() {
    // Ensure directories exist
    await this.ensureDirectories();

    // Spawn Claude CLI
    console.log(`🤖 Starting ${this.agentId} (${this.cliCommand})...`);
    this.spawnClaude();

    // Watch for incoming messages
    this.watchMessages();

    // Send startup announcement
    setTimeout(() => {
      this.sendMessage('*', `${this.agentId} (Claude CLI) is now online!`);
      console.log(`✅ ${this.agentId} ready for messages`);
    }, 1000);
  }

  async ensureDirectories() {
    [this.messagesDir, this.agentDir].forEach(dir => {
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }
    });
  }

  spawnClaude() {
    console.log(`📟 Spawning: ${this.cliCommand}`);

    this.process = spawn(this.cliCommand, [], {
      stdio: ['pipe', 'pipe', 'pipe'],
      cwd: process.cwd(),
      env: {
        ...process.env,
        AGENT_ID: this.agentId,
        NO_COLOR: '1', // Disable ANSI colors
      }
    });

    // Capture ALL stdout
    this.process.stdout.on('data', (data) => {
      this.handleOutput(data);
    });

    // Capture errors
    this.process.stderr.on('data', (data) => {
      const error = data.toString();
      console.error(`❌ [${this.agentId}] Error: ${error}`);
      this.appendToLog(`ERROR: ${error}`);
    });

    // Handle process exit
    this.process.on('exit', (code) => {
      console.log(`💀 [${this.agentId}] Process exited with code ${code}`);
      this.sendMessage('Desktop', `${this.agentId} exited (code: ${code})`);
      process.exit(code);
    });

    console.log(`✅ Claude spawned (PID: ${this.process.pid})`);
    this.updateStatus('running', this.process.pid);
  }

  handleOutput(data) {
    const text = data.toString();

    // Store full output history with size limit
    this.fullOutput += text;

    // Trim buffer if it exceeds max size (keep last 50%)
    if (this.fullOutput.length > this.MAX_OUTPUT_SIZE) {
      const keepSize = Math.floor(this.MAX_OUTPUT_SIZE / 2);
      this.fullOutput = '... [earlier output truncated]\n' + this.fullOutput.slice(-keepSize);
      console.log(`⚠️  [${this.agentId}] Output buffer trimmed (exceeded ${this.MAX_OUTPUT_SIZE} bytes)`);
    }

    // Log to console
    console.log(`[${this.agentId}]`, text);

    // Write to live output file for Desktop UI
    this.writeLiveOutput(text);

    // Append to log file
    this.appendToLog(text);

    // Check if this is a complete response to send
    if (this.shouldSendAsMessage(text)) {
      this.sendOutputAsMessage(text);
    }
  }

  shouldSendAsMessage(text) {
    // Send if it looks like a complete Claude response
    // Skip single-line system messages
    const lines = text.trim().split('\n').filter(l => l.trim());
    return lines.length > 2; // Multi-line responses
  }

  sendOutputAsMessage(text) {
    // Don't spam - debounce multiple rapid outputs
    if (this.outputTimer) clearTimeout(this.outputTimer);

    this.outputTimer = setTimeout(() => {
      const cleanText = text.trim();
      if (cleanText.length > 0) {
        this.sendMessage('Desktop', cleanText);
      }
    }, 500);
  }

  writeLiveOutput(text) {
    const outputFile = path.join(this.agentDir, 'live-output.txt');
    try {
      fs.writeFileSync(outputFile, this.fullOutput);
    } catch (err) {
      console.error('Failed to write live output:', err);
    }
  }

  appendToLog(text) {
    const logFile = path.join(this.agentDir, 'agent.log');
    const timestamp = new Date().toISOString();
    const logEntry = `[${timestamp}] ${text}`;

    try {
      fs.appendFileSync(logFile, logEntry);
    } catch (err) {
      console.error('Failed to append log:', err);
    }
  }

  updateStatus(status, pid = null) {
    // Throttle status writes to max 1 per second (except for explicit status changes)
    const now = Date.now();
    const isExplicitStatusChange = status !== 'running';

    if (!isExplicitStatusChange && (now - this.lastStatusWrite < 1000)) {
      return; // Skip write if less than 1 second since last write
    }

    this.lastStatusWrite = now;

    const statusFile = path.join(this.agentDir, 'status.json');
    const statusData = {
      agentId: this.agentId,
      status: status,
      pid: pid || this.process?.pid,
      startedAt: Date.now(),
      lastUpdate: now,
      messagesReceived: this.processedMessages.size,
      outputLength: this.fullOutput.length
    };

    try {
      fs.writeFileSync(statusFile, JSON.stringify(statusData, null, 2));
    } catch (err) {
      console.error('Failed to write status:', err);
    }
  }

  watchMessages() {
    console.log(`📂 Watching: ${this.messagesDir}`);

    // Get existing messages to skip
    try {
      const existing = fs.readdirSync(this.messagesDir)
        .filter(f => f.endsWith('.json'));
      existing.forEach(f => this.processedMessages.add(f));
      console.log(`ℹ️  Skipping ${existing.length} existing messages`);
    } catch (err) {
      console.error('Failed to read messages dir:', err);
    }

    // Watch for new messages
    fs.watch(this.messagesDir, (eventType, filename) => {
      if (!filename?.endsWith('.json')) return;
      if (this.processedMessages.has(filename)) return;

      // Debounce - file system might trigger multiple events
      setTimeout(() => {
        this.handleNewMessage(filename);
      }, 100);
    });
  }

  handleNewMessage(filename) {
    if (this.processedMessages.has(filename)) return;

    const filePath = path.join(this.messagesDir, filename);

    try {
      // Check if file exists and is readable
      if (!fs.existsSync(filePath)) return;

      const content = fs.readFileSync(filePath, 'utf-8');
      const message = JSON.parse(content);

      // Check if message is for us
      if (!this.isMessageForMe(message)) {
        this.processedMessages.add(filename);
        return;
      }

      // Skip our own messages
      if (message.from.id === this.agentId) {
        this.processedMessages.add(filename);
        return;
      }

      console.log('');
      console.log(`📨 New message from ${message.from.id}:`);
      console.log(`   "${message.payload.text}"`);

      // Add to queue and process
      this.messageQueue.push(message);
      this.processedMessages.add(filename);
      this.processNextMessage();

    } catch (err) {
      console.error(`Error handling message ${filename}:`, err.message);
      this.processedMessages.add(filename); // Mark as processed to avoid retry loop
    }
  }

  isMessageForMe(message) {
    const to = message.to;

    // Exact match
    if (to === this.agentId) return true;

    // Broadcast
    if (to === '*') return true;

    // Wildcard pattern
    if (to.endsWith('*')) {
      const prefix = to.slice(0, -1);
      if (this.agentId.startsWith(prefix)) return true;
    }

    return false;
  }

  async processNextMessage() {
    if (this.isProcessing || this.messageQueue.length === 0) return;

    this.isProcessing = true;
    const message = this.messageQueue.shift();

    try {
      // Inject message into Claude's stdin
      const input = message.payload.text + '\n';
      console.log(`⚡ Injecting into Claude: "${message.payload.text}"`);

      this.process.stdin.write(input);

      // Update status
      this.updateStatus('processing');

    } catch (err) {
      console.error('Error processing message:', err);
    }

    // Ready for next message after a brief delay
    setTimeout(() => {
      this.isProcessing = false;
      this.processNextMessage();
    }, 1000);
  }

  sendMessage(to, text, priority = 'normal') {
    const msgId = `msg-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const message = {
      id: msgId,
      from: {
        id: this.agentId,
        name: `${this.agentId} (Claude)`
      },
      to: to,
      payload: {
        text: text
      },
      timestamp: new Date().toISOString(),
      priority: priority
    };

    const filePath = path.join(this.messagesDir, `${msgId}.json`);

    try {
      fs.writeFileSync(filePath, JSON.stringify(message, null, 2));
      console.log(`✉️  Sent to ${to}`);
      this.processedMessages.add(`${msgId}.json`); // Don't process our own messages
    } catch (err) {
      console.error('Failed to send message:', err);
    }
  }

  stop() {
    console.log(`🛑 Stopping ${this.agentId}...`);
    if (this.process) {
      this.process.kill('SIGTERM');
    }
    this.updateStatus('stopped');
  }
}

// Handle graceful shutdown
function setupShutdownHandlers(agent) {
  const shutdown = () => {
    console.log('');
    console.log('🛑 Shutting down...');
    agent.stop();
    process.exit(0);
  };

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
  process.on('exit', () => agent.stop());
}

// Main
async function main() {
  const agentId = process.argv[2];
  const cliCommand = process.argv[3] || 'claude';

  if (!agentId) {
    console.error('Usage: node reactive-claude-agent.js <agent-id> [cli-command]');
    console.error('Example: node reactive-claude-agent.js Agent1 claude');
    process.exit(1);
  }

  const agent = new ReactiveCLIAgent(agentId, cliCommand);
  setupShutdownHandlers(agent);

  try {
    await agent.start();

    console.log('');
    console.log('💬 Ready for messages...');
    console.log('   Send messages to:', agentId);
    console.log('   Broadcast to all: *');
    console.log('');

  } catch (err) {
    console.error('Failed to start agent:', err);
    process.exit(1);
  }
}

// Export for testing
export { ReactiveCLIAgent };

// Run if called directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}
