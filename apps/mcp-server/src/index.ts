#!/usr/bin/env node

/**
 * AgentMux MCP Server
 *
 * MCP server for inter-agent communication with Claude Code
 * Monitors $HOME/.agentmux/shared/messages/ and sends notifications
 */

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import { MessageBus, AgentIdentity, MessageType, AgentMessage } from '@agentmux/core';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

const SHARED_DIR = path.join(os.homedir(), '.agentmux', 'shared');
const MESSAGES_DIR = path.join(SHARED_DIR, 'messages');

class AgentMuxMCPServer {
  private server: Server;
  private bus: MessageBus;
  private identity: AgentIdentity;
  private lastReadTime: number = Date.now();
  private watchTimer?: NodeJS.Timeout;

  constructor() {
    this.identity = this.detectAgentIdentity();

    this.server = new Server(
      {
        name: 'agentmux',
        version: '0.1.0',
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );

    this.bus = new MessageBus(this.identity);

    this.setupHandlers();
    this.startMessageWatcher();

    // Log startup
    console.error(`[AgentMux MCP] Started as ${this.identity.id}`);
    console.error(`[AgentMux MCP] Watching: ${MESSAGES_DIR}`);
  }

  private detectAgentIdentity(): AgentIdentity {
    const workspace = process.cwd();
    const workspaceName = path.basename(workspace);

    const match = workspaceName.match(/WebProjects(\d+)?$/i);
    const agentName = match ? (match[1] ? `Agent${match[1]}` : 'AgentX') : 'Agent';

    return {
      id: `${agentName}-${process.pid}-${Date.now()}`,
      name: agentName,
      workspace,
      pid: process.pid,
      startedAt: Date.now(),
    };
  }

  private setupHandlers(): void {
    // List available tools
    this.server.setRequestHandler(ListToolsRequestSchema, async () => ({
      tools: [
        {
          name: 'agentmux_send_message',
          description: 'Send a message to another agent or broadcast to all agents',
          inputSchema: {
            type: 'object',
            properties: {
              to: {
                type: 'string',
                description: 'Recipient agent ID, wildcard pattern (e.g., "Agent1-*"), or "*" for broadcast',
              },
              message: {
                type: 'string',
                description: 'Message text to send',
              },
              type: {
                type: 'string',
                description: 'Message type (message, command, status)',
                enum: ['message', 'command', 'status'],
                default: 'message',
              },
            },
            required: ['to', 'message'],
          },
        },
        {
          name: 'agentmux_list_messages',
          description: 'List recent messages from the message bus',
          inputSchema: {
            type: 'object',
            properties: {
              limit: {
                type: 'number',
                description: 'Maximum number of messages to return',
                default: 10,
              },
              type: {
                type: 'string',
                description: 'Filter by message type',
              },
            },
          },
        },
        {
          name: 'agentmux_reply',
          description: 'Reply to a specific message',
          inputSchema: {
            type: 'object',
            properties: {
              messageId: {
                type: 'string',
                description: 'ID of the message to reply to',
              },
              reply: {
                type: 'string',
                description: 'Reply text',
              },
            },
            required: ['messageId', 'reply'],
          },
        },
        {
          name: 'agentmux_get_agents',
          description: 'Get list of active agents (placeholder - requires registry)',
          inputSchema: {
            type: 'object',
            properties: {},
          },
        },
      ],
    }));

    // Handle tool calls
    this.server.setRequestHandler(CallToolRequestSchema, async (request) => {
      const { name, arguments: args } = request.params;

      switch (name) {
        case 'agentmux_send_message': {
          const { to, message, type = 'message' } = args as {
            to: string;
            message: string;
            type?: string;
          };

          const messageId = await this.bus.send(
            to,
            type as MessageType,
            { text: message }
          );

          return {
            content: [
              {
                type: 'text',
                text: `Message sent successfully!\n\nID: ${messageId}\nTo: ${to}\nType: ${type}`,
              },
            ],
          };
        }

        case 'agentmux_list_messages': {
          const { limit = 10, type: filterType } = args as {
            limit?: number;
            type?: string;
          };

          const messages = await this.listMessages(limit, filterType);

          return {
            content: [
              {
                type: 'text',
                text: `Recent messages (${messages.length}):\n\n${JSON.stringify(messages, null, 2)}`,
              },
            ],
          };
        }

        case 'agentmux_reply': {
          const { messageId, reply } = args as {
            messageId: string;
            reply: string;
          };

          // Find original message to get sender
          const originalMessage = await this.findMessageById(messageId);
          if (!originalMessage) {
            return {
              content: [
                {
                  type: 'text',
                  text: `Error: Message ${messageId} not found`,
                },
              ],
              isError: true,
            };
          }

          const replyId = await this.bus.send(
            originalMessage.from.id,
            MessageType.MESSAGE,
            { text: reply },
            messageId
          );

          return {
            content: [
              {
                type: 'text',
                text: `Reply sent successfully!\n\nReply ID: ${replyId}\nTo: ${originalMessage.from.name}\nIn reply to: ${messageId}`,
              },
            ],
          };
        }

        case 'agentmux_get_agents': {
          return {
            content: [
              {
                type: 'text',
                text: 'Active agents:\n(Agent registry not yet implemented - coming in future update)',
              },
            ],
          };
        }

        default:
          throw new Error(`Unknown tool: ${name}`);
      }
    });
  }

  private startMessageWatcher(): void {
    // Watch for new messages and send notifications
    this.watchTimer = setInterval(async () => {
      await this.checkForNewMessages();
    }, 500);

    // Also try fs.watch if available (not reliable on all platforms)
    try {
      fs.watch(MESSAGES_DIR, async (eventType, filename) => {
        if (eventType === 'rename' && filename && filename.endsWith('.json')) {
          await this.checkForNewMessages();
        }
      });
    } catch (err) {
      console.error('[AgentMux MCP] fs.watch not available, using polling only');
    }
  }

  private async checkForNewMessages(): Promise<void> {
    try {
      const files = fs.readdirSync(MESSAGES_DIR);

      for (const file of files) {
        if (!file.endsWith('.json')) continue;

        const filepath = path.join(MESSAGES_DIR, file);
        const stat = fs.statSync(filepath);

        if (stat.mtimeMs <= this.lastReadTime) continue;

        const content = fs.readFileSync(filepath, 'utf8');
        const message: AgentMessage = JSON.parse(content);

        // Skip our own messages
        if (message.from.id === this.identity.id) continue;

        // Check if message is for us
        if (!this.isMessageForMe(message)) continue;

        // Send notification to Claude Code
        await this.sendNotification(message);

        this.lastReadTime = Math.max(this.lastReadTime, stat.mtimeMs);
      }
    } catch (err) {
      console.error('[AgentMux MCP] Error checking messages:', err);
    }
  }

  private isMessageForMe(message: AgentMessage): boolean {
    if (message.to === '*') return true;
    if (message.to === this.identity.id) return true;

    if (typeof message.to === 'string' && message.to.includes('*')) {
      const pattern = message.to.replace(/\*/g, '.*');
      return new RegExp(`^${pattern}$`).test(this.identity.id);
    }

    if (Array.isArray(message.to)) {
      return message.to.includes(this.identity.id);
    }

    return false;
  }

  private async sendNotification(message: AgentMessage): Promise<void> {
    try {
      const payload = message.payload as { text: string };

      await this.server.notification({
        method: 'notifications/message',
        params: {
          level: 'info',
          logger: 'agentmux',
          data: {
            type: 'agentmux_message',
            from: message.from.name,
            fromId: message.from.id,
            message: payload.text || JSON.stringify(message.payload),
            messageType: message.type,
            messageId: message.id,
            timestamp: message.timestamp,
            replyTo: message.replyTo,
          },
        },
      });

      console.error(`[AgentMux MCP] Notification sent for message ${message.id}`);
    } catch (err) {
      console.error('[AgentMux MCP] Error sending notification:', err);
    }
  }

  private async listMessages(limit: number, filterType?: string): Promise<any[]> {
    const files = fs.readdirSync(MESSAGES_DIR)
      .filter(f => f.endsWith('.json'))
      .sort()
      .reverse()
      .slice(0, limit * 2); // Get extra in case we need to filter

    const messages: any[] = [];

    for (const file of files) {
      if (messages.length >= limit) break;

      const filepath = path.join(MESSAGES_DIR, file);
      const content = fs.readFileSync(filepath, 'utf8');
      const message: AgentMessage = JSON.parse(content);

      if (filterType && message.type !== filterType) continue;

      messages.push({
        id: message.id,
        from: message.from.name,
        fromId: message.from.id,
        to: message.to,
        type: message.type,
        message: (message.payload as any).text || message.payload,
        timestamp: new Date(message.timestamp).toLocaleString(),
      });
    }

    return messages;
  }

  private async findMessageById(messageId: string): Promise<AgentMessage | null> {
    const files = fs.readdirSync(MESSAGES_DIR).filter(f => f.includes(messageId));

    if (files.length === 0) return null;

    const filepath = path.join(MESSAGES_DIR, files[0]);
    const content = fs.readFileSync(filepath, 'utf8');
    return JSON.parse(content);
  }

  async start(): Promise<void> {
    const transport = new StdioServerTransport();
    await this.server.connect(transport);
    console.error('[AgentMux MCP] Server connected and ready');
  }

  async stop(): Promise<void> {
    if (this.watchTimer) {
      clearInterval(this.watchTimer);
    }
    await this.bus.stop();
  }
}

// Start server
const server = new AgentMuxMCPServer();
await server.start();

// Handle shutdown
process.on('SIGINT', async () => {
  console.error('[AgentMux MCP] Shutting down...');
  await server.stop();
  process.exit(0);
});

process.on('SIGTERM', async () => {
  console.error('[AgentMux MCP] Shutting down...');
  await server.stop();
  process.exit(0);
});
