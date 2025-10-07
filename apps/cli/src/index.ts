#!/usr/bin/env node

/**
 * AgentMux CLI
 *
 * Command-line tool for inter-agent communication
 */

import { Command } from 'commander';
import chalk from 'chalk';
import { MessageBus, AgentIdentity, MessageType, AgentMessage } from '@agentmux/core';
import * as os from 'os';
import * as path from 'path';
import { findCommandHandler } from './commands';

const program = new Command();

// Create agent identity from environment
function createAgentIdentity(): AgentIdentity {
  const workspace = process.cwd();
  const agentName = process.env.AGENT_NAME || detectAgentName(workspace);
  const pid = process.pid;
  const timestamp = Date.now();

  return {
    id: `${agentName}-${pid}-${timestamp}`,
    name: agentName,
    workspace,
    pid,
    startedAt: timestamp,
  };
}

function detectAgentName(workspace: string): string {
  // Try to detect agent from workspace path
  // E.g., D:\Code\WebProjects1 -> Agent1
  const match = workspace.match(/WebProjects(\d+)?$/i);
  if (match) {
    return match[1] ? `Agent${match[1]}` : 'AgentX';
  }
  return 'Agent';
}

program
  .name('agentmux')
  .description('AgentMux CLI for inter-agent communication')
  .version('0.1.0');

// Send command
program
  .command('send')
  .description('Send a message to another agent')
  .argument('<to>', 'recipient agent ID or "*" for broadcast')
  .argument('<message>', 'message text')
  .option('-t, --type <type>', 'message type', 'message')
  .action(async (to: string, message: string, options: { type: string }) => {
    const identity = createAgentIdentity();
    const bus = new MessageBus(identity);

    console.log(chalk.blue(`📤 Sending from ${identity.id}...`));

    try {
      const messageId = await bus.send(
        to,
        options.type as MessageType,
        { text: message }
      );

      console.log(chalk.green(`✓ Message sent (ID: ${messageId})`));
      console.log(chalk.gray(`  To: ${to}`));
      console.log(chalk.gray(`  Type: ${options.type}`));

      process.exit(0);
    } catch (err) {
      console.error(chalk.red(`✗ Failed to send message:`), err);
      process.exit(1);
    }
  });

// Listen command
program
  .command('listen')
  .description('Listen for incoming messages')
  .option('-t, --type <type>', 'filter by message type')
  .action((options: { type?: string }) => {
    const identity = createAgentIdentity();
    const bus = new MessageBus(identity);

    console.log(chalk.blue(`📡 Listening as ${identity.id}...`));
    console.log(chalk.gray(`  Workspace: ${identity.workspace}`));
    console.log(chalk.gray(`  PID: ${identity.pid}`));
    console.log(chalk.gray(`  Press Ctrl+C to stop\n`));

    // Handle all messages
    bus.on('*', async (message: AgentMessage) => {
      if (options.type && message.type !== options.type) {
        return; // Skip if filtering and type doesn't match
      }

      console.log(chalk.yellow(`\n📨 Message received:`));
      console.log(chalk.gray(`  From: ${message.from.name} (${message.from.id})`));
      console.log(chalk.gray(`  Type: ${message.type}`));
      console.log(chalk.gray(`  Time: ${new Date(message.timestamp).toLocaleTimeString()}`));

      if (message.type === MessageType.MESSAGE) {
        const payload = message.payload as { text: string };
        console.log(chalk.white(`\n  ${payload.text}\n`));
      } else if (message.type === MessageType.COMMAND) {
        const payload = message.payload as { text: string };
        console.log(chalk.cyan(`\n  Command: ${payload.text}`));

        // Check for built-in commands
        const handler = findCommandHandler(payload.text);
        if (handler) {
          console.log(chalk.gray(`  Auto-responding...`));
          await handler.handler(message, bus);
          console.log(chalk.green(`  ✓ Response sent\n`));
        } else {
          console.log(chalk.gray(`  (No handler for this command)\n`));
        }
      } else {
        console.log(chalk.gray(`  Payload: ${JSON.stringify(message.payload, null, 2)}\n`));
      }
    });

    bus.start();

    // Handle shutdown
    process.on('SIGINT', async () => {
      console.log(chalk.yellow('\n\n👋 Shutting down...'));
      await bus.stop();
      process.exit(0);
    });
  });

// List command
program
  .command('list')
  .description('List active agents')
  .action(() => {
    console.log(chalk.blue('📋 Active agents:'));
    console.log(chalk.gray('  (Not implemented yet - requires registry)'));
  });

// Status command
program
  .command('status')
  .description('Show status of message bus')
  .action(() => {
    const busPath = path.join(process.cwd(), '_temp', 'agentmux-bus');
    console.log(chalk.blue('📊 Message Bus Status:'));
    console.log(chalk.gray(`  Bus path: ${busPath}`));
    console.log(chalk.gray(`  Transport: file`));
  });

program.parse();
