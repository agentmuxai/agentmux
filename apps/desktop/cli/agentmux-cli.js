#!/usr/bin/env node

/**
 * AgentMux Desktop CLI
 * Command-line interface to control the running AgentMux Desktop app
 */

import fs from 'fs';
import path from 'path';
import os from 'os';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const VERSION = '0.1.0';
const CONFIG_DIR = path.join(os.homedir(), '.agentmux', 'desktop');
const MESSAGES_DIR = path.join(os.homedir(), '.agentmux', 'shared', 'messages');
const COMMANDS_DIR = path.join(os.homedir(), '.agentmux', 'desktop', 'commands');

// Ensure directories exist
[CONFIG_DIR, MESSAGES_DIR, COMMANDS_DIR].forEach(dir => {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
});

/**
 * Send a message to another agent
 */
function sendMessage(to, text, priority = 'normal') {
  const msgId = `msg-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const message = {
    id: msgId,
    from: {
      id: 'CLI',
      name: 'AgentMux CLI'
    },
    to: to,
    payload: {
      text: text
    },
    timestamp: new Date().toISOString(),
    priority: priority
  };

  const filePath = path.join(MESSAGES_DIR, `${msgId}.json`);
  fs.writeFileSync(filePath, JSON.stringify(message, null, 2));

  console.log(`✉️  Message sent to ${to}`);
  console.log(`   ID: ${msgId}`);
  console.log(`   Text: "${text}"`);
  console.log(`   Priority: ${priority}`);
  return msgId;
}

/**
 * Send a command to the Desktop app
 */
function sendCommand(command, params = {}) {
  const cmdId = `cmd-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const commandData = {
    id: cmdId,
    command: command,
    params: params,
    timestamp: new Date().toISOString()
  };

  const filePath = path.join(COMMANDS_DIR, `${cmdId}.json`);
  fs.writeFileSync(filePath, JSON.stringify(commandData, null, 2));

  console.log(`🎛️  Command sent: ${command}`);
  console.log(`   ID: ${cmdId}`);
  if (Object.keys(params).length > 0) {
    console.log(`   Params: ${JSON.stringify(params)}`);
  }
  return cmdId;
}

/**
 * List recent messages
 */
function listMessages(limit = 10) {
  const files = fs.readdirSync(MESSAGES_DIR)
    .filter(f => f.endsWith('.json'))
    .sort()
    .reverse()
    .slice(0, limit);

  if (files.length === 0) {
    console.log('📭 No messages found');
    return;
  }

  console.log(`📬 Last ${files.length} messages:\n`);

  files.forEach((file, idx) => {
    const content = fs.readFileSync(path.join(MESSAGES_DIR, file), 'utf-8');
    const msg = JSON.parse(content);

    console.log(`${idx + 1}. ${msg.from.id} → ${msg.to}`);
    console.log(`   "${msg.payload.text}"`);
    console.log(`   ${msg.timestamp} [${msg.priority}]`);
    console.log(`   ID: ${msg.id}`);
    console.log('');
  });
}

/**
 * Get status information
 */
function getStatus() {
  // Check if messages directory has recent activity
  const files = fs.readdirSync(MESSAGES_DIR).filter(f => f.endsWith('.json'));
  const latestFile = files.sort().reverse()[0];

  let lastActivity = 'Never';
  if (latestFile) {
    const stats = fs.statSync(path.join(MESSAGES_DIR, latestFile));
    lastActivity = stats.mtime.toLocaleString();
  }

  console.log('📊 AgentMux Desktop Status\n');
  console.log(`Version: ${VERSION}`);
  console.log(`Messages Dir: ${MESSAGES_DIR}`);
  console.log(`Commands Dir: ${COMMANDS_DIR}`);
  console.log(`Total Messages: ${files.length}`);
  console.log(`Last Activity: ${lastActivity}`);
  console.log('');

  // Check if desktop app is likely running (recent file activity)
  const recentThreshold = 60 * 1000; // 1 minute
  if (latestFile) {
    const stats = fs.statSync(path.join(MESSAGES_DIR, latestFile));
    const age = Date.now() - stats.mtime.getTime();

    if (age < recentThreshold) {
      console.log('✅ Desktop app appears to be active (recent message activity)');
    } else {
      console.log('⚠️  No recent activity detected (app may not be running)');
    }
  } else {
    console.log('⚠️  No messages found (app may not be running)');
  }
}

/**
 * Start the bus
 */
function startBus(host = '127.0.0.1', port = 8765) {
  return sendCommand('start_bus', { host, port, max_agents: 50 });
}

/**
 * Stop the bus
 */
function stopBus() {
  return sendCommand('stop_bus');
}

/**
 * Start file watcher
 */
function startWatcher(agentId = null) {
  return sendCommand('start_file_watcher', { agent_id: agentId });
}

/**
 * Stop file watcher
 */
function stopWatcher() {
  return sendCommand('stop_file_watcher');
}

/**
 * Display help
 */
function showHelp() {
  console.log(`
AgentMux Desktop CLI v${VERSION}

Usage: agentmux-cli <command> [options]

Commands:

  Messages:
    send <to> <message>           Send a message to an agent
    send <to> <message> --urgent  Send urgent priority message
    list [limit]                  List recent messages (default: 10)

  Control:
    start-bus [host] [port]       Start WebSocket bus (default: 127.0.0.1:8765)
    stop-bus                      Stop WebSocket bus
    start-watcher [agent-id]      Start file watcher (optional agent ID filter)
    stop-watcher                  Stop file watcher

  Info:
    status                        Show status information
    version                       Show version
    help                          Show this help message

Examples:

  # Send a message to Agent1
  agentmux-cli send Agent1 "Hello from CLI!"

  # Send urgent message to all agents
  agentmux-cli send "*" "System alert!" --urgent

  # List last 20 messages
  agentmux-cli list 20

  # Start the WebSocket bus
  agentmux-cli start-bus

  # Start file watcher for specific agent
  agentmux-cli start-watcher Agent1

  # Check status
  agentmux-cli status

Notes:
  - Desktop app must be running to receive commands
  - Commands are sent via files in ~/.agentmux/desktop/commands/
  - Messages are sent via files in ~/.agentmux/shared/messages/

Config Directory: ${CONFIG_DIR}
Messages Directory: ${MESSAGES_DIR}
Commands Directory: ${COMMANDS_DIR}
`);
}

/**
 * Main CLI handler
 */
function main() {
  const args = process.argv.slice(2);

  if (args.length === 0) {
    showHelp();
    process.exit(0);
  }

  const command = args[0];

  try {
    switch (command) {
      case 'send':
        if (args.length < 3) {
          console.error('❌ Error: Missing arguments');
          console.log('Usage: agentmux-cli send <to> <message> [--urgent]');
          process.exit(1);
        }
        const to = args[1];
        const isUrgent = args.includes('--urgent');
        const messageArgs = args.slice(2).filter(arg => arg !== '--urgent');
        const message = messageArgs.join(' ');
        const priority = isUrgent ? 'urgent' : 'normal';
        sendMessage(to, message, priority);
        break;

      case 'list':
        const limit = parseInt(args[1]) || 10;
        listMessages(limit);
        break;

      case 'start-bus':
        const host = args[1] || '127.0.0.1';
        const port = parseInt(args[2]) || 8765;
        startBus(host, port);
        console.log(`⏳ Command queued. Desktop app will start bus on ${host}:${port}`);
        break;

      case 'stop-bus':
        stopBus();
        console.log('⏳ Command queued. Desktop app will stop bus.');
        break;

      case 'start-watcher':
        const agentId = args[1] || null;
        startWatcher(agentId);
        console.log(`⏳ Command queued. Desktop app will start file watcher${agentId ? ` for ${agentId}` : ''}.`);
        break;

      case 'stop-watcher':
        stopWatcher();
        console.log('⏳ Command queued. Desktop app will stop file watcher.');
        break;

      case 'status':
        getStatus();
        break;

      case 'version':
        console.log(`AgentMux Desktop CLI v${VERSION}`);
        break;

      case 'help':
      case '--help':
      case '-h':
        showHelp();
        break;

      default:
        console.error(`❌ Unknown command: ${command}`);
        console.log('Run "agentmux-cli help" for usage information.');
        process.exit(1);
    }
  } catch (error) {
    console.error('❌ Error:', error.message);
    process.exit(1);
  }
}

// Run CLI
main();
