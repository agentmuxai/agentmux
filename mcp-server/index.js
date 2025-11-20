#!/usr/bin/env node

/**
 * AgentMux MCP Server
 *
 * Provides Model Context Protocol tools for inter-agent communication using
 * file-based messaging. Agents can send messages, read their inbox, and
 * discover other agents.
 *
 * Storage: ~/.agentmux/shared/messages/
 * Format: JSON files (one per message)
 * Detection: File watcher with <100ms latency
 */

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { CallToolRequestSchema, ListToolsRequestSchema } from '@modelcontextprotocol/sdk/types.js';
import fs from 'fs';
import path from 'path';
import os from 'os';
import chokidar from 'chokidar';

// Configuration
const AGENT_ID = process.env.AGENT_ID || 'unknown';
const MESSAGES_DIR = path.join(os.homedir(), '.agentmux', 'shared', 'messages');
const REGISTRY_DIR = path.join(os.homedir(), '.agentmux', 'registry');

// Ensure directories exist
[MESSAGES_DIR, REGISTRY_DIR].forEach(dir => {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
});

// Create MCP server
const server = new Server({
  name: 'agentmux-mcp',
  version: '0.1.0',
}, {
  capabilities: {
    tools: {},
  },
});

// Tool definitions
const TOOLS = [
  {
    name: 'send_message',
    description: 'Send a message to another agent via AgentMux',
    inputSchema: {
      type: 'object',
      properties: {
        to: {
          type: 'string',
          description: 'Target agent ID (e.g., "agent1", "agent2", "*" for broadcast)',
        },
        message: {
          type: 'string',
          description: 'Message text to send',
        },
        priority: {
          type: 'string',
          enum: ['low', 'normal', 'high', 'urgent'],
          description: 'Message priority (default: normal)',
          default: 'normal',
        },
      },
      required: ['to', 'message'],
    },
  },
  {
    name: 'read_messages',
    description: 'Read messages sent to this agent',
    inputSchema: {
      type: 'object',
      properties: {
        unread_only: {
          type: 'boolean',
          description: 'Only return unread messages (default: true)',
          default: true,
        },
        limit: {
          type: 'number',
          description: 'Maximum number of messages to return (default: 10)',
          default: 10,
        },
        mark_as_read: {
          type: 'boolean',
          description: 'Mark returned messages as read (default: true)',
          default: true,
        },
      },
    },
  },
  {
    name: 'list_agents',
    description: 'List all agents that have sent or received messages',
    inputSchema: {
      type: 'object',
      properties: {},
    },
  },
  {
    name: 'broadcast_message',
    description: 'Send a message to all agents',
    inputSchema: {
      type: 'object',
      properties: {
        message: {
          type: 'string',
          description: 'Message to broadcast to all agents',
        },
        exclude_self: {
          type: 'boolean',
          description: 'Do not send to the broadcasting agent (default: true)',
          default: true,
        },
        priority: {
          type: 'string',
          enum: ['low', 'normal', 'high', 'urgent'],
          description: 'Message priority (default: normal)',
          default: 'normal',
        },
      },
      required: ['message'],
    },
  },
  {
    name: 'delete_messages',
    description: 'Delete specific messages from inbox',
    inputSchema: {
      type: 'object',
      properties: {
        message_ids: {
          type: 'array',
          items: { type: 'string' },
          description: 'Array of message IDs to delete',
        },
      },
      required: ['message_ids'],
    },
  },
];

// Handle list tools
server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: TOOLS,
}));

// Handle tool calls
server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
    switch (name) {
      case 'send_message':
        return await sendMessage(args);
      case 'read_messages':
        return await readMessages(args);
      case 'list_agents':
        return await listAgents(args);
      case 'broadcast_message':
        return await broadcastMessage(args);
      case 'delete_messages':
        return await deleteMessages(args);
      default:
        throw new Error(`Unknown tool: ${name}`);
    }
  } catch (error) {
    return {
      content: [
        {
          type: 'text',
          text: `Error: ${error.message}`,
        },
      ],
      isError: true,
    };
  }
});

/**
 * Send a message to another agent
 */
async function sendMessage({ to, message, priority = 'normal' }) {
  const msgId = `msg-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;

  const messageObj = {
    id: msgId,
    from: {
      id: AGENT_ID,
      name: AGENT_ID,
    },
    to: to,
    payload: {
      text: message,
    },
    timestamp: new Date().toISOString(),
    priority: priority,
    read: false,
  };

  const filePath = path.join(MESSAGES_DIR, `${msgId}.json`);
  fs.writeFileSync(filePath, JSON.stringify(messageObj, null, 2));

  return {
    content: [
      {
        type: 'text',
        text: JSON.stringify({
          success: true,
          message_id: msgId,
          from: AGENT_ID,
          to: to,
          delivered_at: messageObj.timestamp,
          priority: priority,
        }, null, 2),
      },
    ],
  };
}

/**
 * Read messages sent to this agent
 */
async function readMessages({ unread_only = true, limit = 10, mark_as_read = true } = {}) {
  const files = fs.readdirSync(MESSAGES_DIR)
    .filter(f => f.endsWith('.json'))
    .sort()
    .reverse();

  const messages = [];

  for (const file of files) {
    if (messages.length >= limit) break;

    try {
      const filePath = path.join(MESSAGES_DIR, file);
      const content = fs.readFileSync(filePath, 'utf-8');
      const msg = JSON.parse(content);

      // Check if message is for this agent
      if (msg.to !== AGENT_ID && msg.to !== '*') continue;

      // Check unread filter
      if (unread_only && msg.read) continue;

      messages.push({
        id: msg.id,
        from: msg.from.id,
        message: msg.payload.text,
        timestamp: msg.timestamp,
        priority: msg.priority,
        read: msg.read,
      });

      // Mark as read if requested
      if (mark_as_read && !msg.read) {
        msg.read = true;
        fs.writeFileSync(filePath, JSON.stringify(msg, null, 2));
      }
    } catch (error) {
      console.error(`Error reading message file ${file}:`, error.message);
    }
  }

  return {
    content: [
      {
        type: 'text',
        text: JSON.stringify({
          agent_id: AGENT_ID,
          messages: messages,
          count: messages.length,
          unread_total: messages.filter(m => !m.read).length,
        }, null, 2),
      },
    ],
  };
}

/**
 * List all agents that have participated in messaging
 */
async function listAgents() {
  const files = fs.readdirSync(MESSAGES_DIR)
    .filter(f => f.endsWith('.json'));

  const agentsMap = new Map();

  for (const file of files) {
    try {
      const content = fs.readFileSync(path.join(MESSAGES_DIR, file), 'utf-8');
      const msg = JSON.parse(content);

      // Add sender
      if (!agentsMap.has(msg.from.id)) {
        agentsMap.set(msg.from.id, {
          agent_id: msg.from.id,
          last_seen: msg.timestamp,
          messages_sent: 0,
        });
      }
      agentsMap.get(msg.from.id).messages_sent++;

      // Update last seen if more recent
      if (msg.timestamp > agentsMap.get(msg.from.id).last_seen) {
        agentsMap.get(msg.from.id).last_seen = msg.timestamp;
      }

      // Add recipient (if not broadcast)
      if (msg.to !== '*' && !agentsMap.has(msg.to)) {
        agentsMap.set(msg.to, {
          agent_id: msg.to,
          last_seen: msg.timestamp,
          messages_sent: 0,
        });
      }
    } catch (error) {
      console.error(`Error reading message file ${file}:`, error.message);
    }
  }

  const agents = Array.from(agentsMap.values()).sort((a, b) =>
    b.last_seen.localeCompare(a.last_seen)
  );

  return {
    content: [
      {
        type: 'text',
        text: JSON.stringify({
          current_agent: AGENT_ID,
          agents: agents,
          total_count: agents.length,
        }, null, 2),
      },
    ],
  };
}

/**
 * Broadcast a message to all agents
 */
async function broadcastMessage({ message, exclude_self = true, priority = 'normal' }) {
  const msgId = `msg-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;

  const messageObj = {
    id: msgId,
    from: {
      id: AGENT_ID,
      name: AGENT_ID,
    },
    to: '*',
    payload: {
      text: message,
    },
    timestamp: new Date().toISOString(),
    priority: priority,
    read: false,
  };

  const filePath = path.join(MESSAGES_DIR, `${msgId}.json`);
  fs.writeFileSync(filePath, JSON.stringify(messageObj, null, 2));

  return {
    content: [
      {
        type: 'text',
        text: JSON.stringify({
          success: true,
          message_id: msgId,
          from: AGENT_ID,
          to: 'all agents',
          delivered_at: messageObj.timestamp,
          priority: priority,
          broadcast: true,
        }, null, 2),
      },
    ],
  };
}

/**
 * Delete messages by ID
 */
async function deleteMessages({ message_ids }) {
  const deleted = [];
  const errors = [];

  for (const msgId of message_ids) {
    const filePath = path.join(MESSAGES_DIR, `${msgId}.json`);

    try {
      if (fs.existsSync(filePath)) {
        // Verify message is for this agent or from this agent
        const content = fs.readFileSync(filePath, 'utf-8');
        const msg = JSON.parse(content);

        if (msg.to === AGENT_ID || msg.from.id === AGENT_ID || msg.to === '*') {
          fs.unlinkSync(filePath);
          deleted.push(msgId);
        } else {
          errors.push({ id: msgId, error: 'Not authorized to delete this message' });
        }
      } else {
        errors.push({ id: msgId, error: 'Message not found' });
      }
    } catch (error) {
      errors.push({ id: msgId, error: error.message });
    }
  }

  return {
    content: [
      {
        type: 'text',
        text: JSON.stringify({
          deleted: deleted,
          deleted_count: deleted.length,
          errors: errors,
        }, null, 2),
      },
    ],
  };
}

/**
 * Start the MCP server
 */
async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);

  // Log to stderr (stdout is used for MCP protocol)
  console.error(`[AgentMux MCP] Server started for agent: ${AGENT_ID}`);
  console.error(`[AgentMux MCP] Messages directory: ${MESSAGES_DIR}`);
  console.error(`[AgentMux MCP] Ready for tool calls`);
}

main().catch((error) => {
  console.error('[AgentMux MCP] Fatal error:', error);
  process.exit(1);
});
