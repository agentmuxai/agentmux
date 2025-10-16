#!/usr/bin/env node

/**
 * Visual Wrapper Demo - Shows wrapped Claude CLI with clear border
 *
 * This wrapper makes it VISUALLY OBVIOUS that Claude CLI is wrapped:
 * - High contrast border around Claude's output
 * - Shows wrapper status messages
 * - Displays incoming messages
 * - Clear separation between wrapper and Claude
 */

import { spawn } from 'child_process';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// ANSI color codes for visual distinction
const colors = {
  reset: '\x1b[0m',
  bright: '\x1b[1m',
  dim: '\x1b[2m',

  // Wrapper colors (high contrast)
  wrapperBorder: '\x1b[1;36m',  // Bright cyan
  wrapperText: '\x1b[1;33m',     // Bright yellow
  wrapperInfo: '\x1b[1;32m',     // Bright green
  wrapperError: '\x1b[1;31m',    // Bright red

  // Claude output (normal)
  claudeOutput: '\x1b[0m',       // Normal/reset

  // Message colors
  messageHeader: '\x1b[1;35m',   // Bright magenta
  messageBody: '\x1b[0;35m',     // Normal magenta
};

const border = {
  top: '═',
  bottom: '═',
  left: '║',
  right: '║',
  topLeft: '╔',
  topRight: '╗',
  bottomLeft: '╚',
  bottomRight: '╝',
};

class VisualWrapperDemo {
  constructor(agentId, cliCommand = 'claude') {
    this.agentId = agentId;
    this.cliCommand = cliCommand;
    this.process = null;
    this.messagesDir = path.join(os.homedir(), '.agentmux', 'shared', 'messages');
    this.processedMessages = new Set();
    this.terminalWidth = process.stdout.columns || 80;
  }

  printBorder(char, text = '') {
    const width = this.terminalWidth;
    if (text) {
      const padding = Math.max(0, width - text.length - 4);
      const leftPad = Math.floor(padding / 2);
      const rightPad = padding - leftPad;
      console.log(
        colors.wrapperBorder +
        border.topLeft +
        border.top.repeat(leftPad) +
        ' ' + colors.wrapperText + text + colors.wrapperBorder + ' ' +
        border.top.repeat(rightPad) +
        border.topRight +
        colors.reset
      );
    } else {
      console.log(
        colors.wrapperBorder +
        char.repeat(width) +
        colors.reset
      );
    }
  }

  printWrapperLine(text, color = colors.wrapperInfo) {
    const width = this.terminalWidth;
    const contentWidth = width - 4; // Account for borders and spaces
    const padding = ' '.repeat(Math.max(0, contentWidth - text.length));
    console.log(
      colors.wrapperBorder + border.left + ' ' +
      color + text + padding +
      colors.wrapperBorder + ' ' + border.right +
      colors.reset
    );
  }

  printClaudeLine(text) {
    // Claude's output - just pass through with normal formatting
    console.log(colors.claudeOutput + text + colors.reset);
  }

  printMessageBox(from, message) {
    const width = this.terminalWidth;

    console.log(); // Blank line
    this.printBorder(border.top, '📨 INCOMING MESSAGE');
    this.printWrapperLine('', colors.messageHeader);
    this.printWrapperLine(`From: ${from}`, colors.messageHeader);
    this.printWrapperLine(`Time: ${new Date().toLocaleTimeString()}`, colors.messageHeader);
    this.printWrapperLine('', colors.messageHeader);

    // Split message into lines that fit
    const contentWidth = width - 6;
    const words = message.split(' ');
    let currentLine = '';

    for (const word of words) {
      if ((currentLine + word).length > contentWidth) {
        if (currentLine) {
          this.printWrapperLine(currentLine.trim(), colors.messageBody);
        }
        currentLine = word + ' ';
      } else {
        currentLine += word + ' ';
      }
    }
    if (currentLine.trim()) {
      this.printWrapperLine(currentLine.trim(), colors.messageBody);
    }

    this.printWrapperLine('', colors.messageHeader);
    this.printBorder(border.bottom, '');
    console.log(); // Blank line
  }

  async start() {
    // Ensure directories exist
    if (!fs.existsSync(this.messagesDir)) {
      fs.mkdirSync(this.messagesDir, { recursive: true });
    }

    // Print wrapper startup
    console.clear();
    this.printBorder(border.top, 'AGENTMUX WRAPPER ACTIVE');
    this.printWrapperLine('', colors.wrapperInfo);
    this.printWrapperLine(`Agent ID: ${this.agentId}`, colors.wrapperInfo);
    this.printWrapperLine(`CLI Command: ${this.cliCommand}`, colors.wrapperInfo);
    this.printWrapperLine(`Messages: ${this.messagesDir}`, colors.wrapperInfo);
    this.printWrapperLine('', colors.wrapperInfo);
    this.printWrapperLine('Spawning Claude CLI...', colors.wrapperInfo);
    this.printBorder(border.bottom, '');
    console.log();

    // Spawn Claude CLI
    this.spawnClaude();

    // Watch for messages
    this.watchMessages();

    // Print status every 10 seconds
    setInterval(() => {
      this.printStatus();
    }, 10000);
  }

  printStatus() {
    console.log();
    this.printBorder(border.top, 'WRAPPER STATUS');
    this.printWrapperLine(`Agent: ${this.agentId}`, colors.wrapperInfo);
    this.printWrapperLine(`PID: ${this.process?.pid || 'N/A'}`, colors.wrapperInfo);
    this.printWrapperLine(`Messages Processed: ${this.processedMessages.size}`, colors.wrapperInfo);
    this.printWrapperLine(`Watching: ${this.messagesDir}`, colors.wrapperInfo);
    this.printBorder(border.bottom, '');
    console.log();
  }

  spawnClaude() {
    this.process = spawn(this.cliCommand, [], {
      stdio: ['pipe', 'pipe', 'pipe'],
      cwd: process.cwd(),
      env: {
        ...process.env,
        AGENT_ID: this.agentId,
        NO_COLOR: '0', // Allow Claude's colors
      }
    });

    // Capture stdout - this is Claude's output
    this.process.stdout.on('data', (data) => {
      const text = data.toString();

      // Print with visual wrapper border on each line
      const lines = text.split('\n');
      lines.forEach((line, i) => {
        if (i === 0) {
          // First line - add top border
          console.log(colors.wrapperBorder + '│ ' + colors.reset);
        }

        // Claude's actual output
        if (line.trim()) {
          console.log(
            colors.wrapperBorder + '│ ' +
            colors.claudeOutput + line +
            colors.reset
          );
        }
      });
    });

    // Capture stderr
    this.process.stderr.on('data', (data) => {
      const error = data.toString();
      this.printWrapperLine(`[ERROR] ${error}`, colors.wrapperError);
    });

    // Handle exit
    this.process.on('exit', (code) => {
      console.log();
      this.printBorder(border.top, 'CLAUDE CLI EXITED');
      this.printWrapperLine(`Exit Code: ${code}`, colors.wrapperError);
      this.printBorder(border.bottom, '');
      process.exit(code);
    });

    this.printWrapperLine(`✓ Claude spawned (PID: ${this.process.pid})`, colors.wrapperInfo);
  }

  watchMessages() {
    // Get existing messages to skip
    try {
      const existing = fs.readdirSync(this.messagesDir)
        .filter(f => f.endsWith('.json'));
      existing.forEach(f => this.processedMessages.add(f));

      this.printWrapperLine(`Skipping ${existing.length} existing messages`, colors.wrapperInfo);
    } catch (err) {
      this.printWrapperLine(`Error reading messages: ${err.message}`, colors.wrapperError);
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

      // Display message in visual box
      this.printMessageBox(
        message.from.name || message.from.id,
        message.payload.text
      );

      // Inject into Claude's stdin
      const input = message.payload.text + '\n';
      this.printWrapperLine(`⚡ Injecting message into Claude CLI...`, colors.wrapperInfo);
      this.process.stdin.write(input);

      this.processedMessages.add(filename);

    } catch (err) {
      this.printWrapperLine(`Error handling message: ${err.message}`, colors.wrapperError);
      this.processedMessages.add(filename);
    }
  }

  isMessageForMe(message) {
    const to = message.to;

    if (to === this.agentId) return true;
    if (to === '*') return true;

    if (to.endsWith('*')) {
      const prefix = to.slice(0, -1);
      if (this.agentId.startsWith(prefix)) return true;
    }

    return false;
  }

  stop() {
    console.log();
    this.printBorder(border.top, 'STOPPING WRAPPER');
    this.printWrapperLine('Shutting down...', colors.wrapperInfo);
    this.printBorder(border.bottom, '');

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
  process.on('exit', () => wrapper.stop());
}

// Main
async function main() {
  const agentId = process.argv[2];
  const cliCommand = process.argv[3] || 'claude';

  if (!agentId) {
    console.error('Usage: node visual-wrapper-demo.js <agent-id> [cli-command]');
    console.error('Example: node visual-wrapper-demo.js Agent2 claude');
    process.exit(1);
  }

  const wrapper = new VisualWrapperDemo(agentId, cliCommand);
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

export { VisualWrapperDemo };
