/**
 * AgentMux Message Bus
 *
 * File-based message bus for inter-agent communication
 * Simple MVP using filesystem as transport
 */

import { nanoid } from 'nanoid';
import * as fs from 'fs';
import * as path from 'path';
import { AgentMessage, AgentIdentity, MessageBusConfig, MessageHandler, MessageType } from './types';

export class MessageBus {
  private config: MessageBusConfig;
  private identity: AgentIdentity;
  private handlers: Map<MessageType | '*', MessageHandler[]> = new Map();
  private pollTimer?: NodeJS.Timeout;
  private lastReadTime: number = Date.now();
  private busPath: string;
  private inboxPath: string;
  private outboxPath: string;

  constructor(identity: AgentIdentity, config?: Partial<MessageBusConfig>) {
    this.identity = identity;
    this.config = {
      transport: 'file',
      busPath: path.join(process.cwd(), '_temp', 'agentmux-bus'),
      pollInterval: 1000,
      ...config,
    };

    if (this.config.transport !== 'file') {
      throw new Error('Only file transport is supported in MVP');
    }

    this.busPath = this.config.busPath!;
    this.inboxPath = path.join(this.busPath, 'inbox');
    this.outboxPath = path.join(this.busPath, 'outbox');

    this.ensureBusDirectories();
  }

  private ensureBusDirectories(): void {
    [this.busPath, this.inboxPath, this.outboxPath].forEach((dir) => {
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }
    });
  }

  /**
   * Register a message handler
   */
  on(type: MessageType | '*', handler: MessageHandler): void {
    const handlers = this.handlers.get(type) || [];
    handlers.push(handler);
    this.handlers.set(type, handlers);
  }

  /**
   * Remove a message handler
   */
  off(type: MessageType | '*', handler: MessageHandler): void {
    const handlers = this.handlers.get(type);
    if (handlers) {
      const index = handlers.indexOf(handler);
      if (index > -1) {
        handlers.splice(index, 1);
      }
    }
  }

  /**
   * Send a message
   */
  async send(to: string | string[], type: MessageType, payload: unknown, replyTo?: string): Promise<string> {
    const message: AgentMessage = {
      id: nanoid(),
      from: this.identity,
      to,
      type,
      payload,
      timestamp: Date.now(),
      replyTo,
    };

    const filename = `${message.timestamp}-${message.id}.json`;
    const filepath = path.join(this.outboxPath, filename);

    fs.writeFileSync(filepath, JSON.stringify(message, null, 2));

    return message.id;
  }

  /**
   * Broadcast a message to all agents
   */
  async broadcast(type: MessageType, payload: unknown): Promise<string> {
    return this.send('*', type, payload);
  }

  /**
   * Start listening for messages
   */
  start(): void {
    if (this.pollTimer) {
      return; // Already started
    }

    // Register this agent
    this.broadcast(MessageType.REGISTER, {
      workspace: this.identity.workspace,
      pid: this.identity.pid,
      startedAt: this.identity.startedAt,
    });

    // Start polling for messages
    this.pollTimer = setInterval(() => {
      this.pollMessages();
    }, this.config.pollInterval);
  }

  /**
   * Stop listening for messages
   */
  async stop(): Promise<void> {
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = undefined;
    }

    // Notify other agents of shutdown
    await this.broadcast(MessageType.SHUTDOWN, {
      reason: 'normal',
    });
  }

  private pollMessages(): void {
    try {
      const files = fs.readdirSync(this.inboxPath);

      for (const file of files) {
        if (!file.endsWith('.json')) continue;

        const filepath = path.join(this.inboxPath, file);
        const stat = fs.statSync(filepath);

        // Only process messages newer than last read
        if (stat.mtimeMs <= this.lastReadTime) continue;

        try {
          const content = fs.readFileSync(filepath, 'utf8');
          const message: AgentMessage = JSON.parse(content);

          // Skip our own messages
          if (message.from.id === this.identity.id) continue;

          // Check if message is for us
          if (message.to !== '*' && message.to !== this.identity.id) {
            if (Array.isArray(message.to) && !message.to.includes(this.identity.id)) {
              continue;
            }
          }

          this.handleMessage(message);

          // Update last read time
          this.lastReadTime = Math.max(this.lastReadTime, stat.mtimeMs);
        } catch (err) {
          console.error(`Error reading message ${file}:`, err);
        }
      }
    } catch (err) {
      console.error('Error polling messages:', err);
    }
  }

  private handleMessage(message: AgentMessage): void {
    // Call type-specific handlers
    const typeHandlers = this.handlers.get(message.type) || [];
    typeHandlers.forEach((handler) => {
      try {
        handler(message);
      } catch (err) {
        console.error(`Error in handler for ${message.type}:`, err);
      }
    });

    // Call wildcard handlers
    const wildcardHandlers = this.handlers.get('*') || [];
    wildcardHandlers.forEach((handler) => {
      try {
        handler(message);
      } catch (err) {
        console.error('Error in wildcard handler:', err);
      }
    });
  }
}
