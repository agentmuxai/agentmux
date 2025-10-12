/**
 * AgentMux Message Bus
 *
 * File-based message bus for inter-agent communication
 * Uses shared $HOME/.agentmux/shared/ directory for cross-workspace communication
 */

import { nanoid } from 'nanoid';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { AgentMessage, AgentIdentity, MessageBusConfig, MessageHandler, MessageType } from './types';

export class MessageBus {
  private config: MessageBusConfig;
  private identity: AgentIdentity;
  private handlers: Map<MessageType | '*', MessageHandler[]> = new Map();
  private pollTimer?: NodeJS.Timeout;
  private lastReadTime: number = Date.now();
  private busPath: string;
  private messagesPath: string;
  private agentsPath: string;

  constructor(identity: AgentIdentity, config?: Partial<MessageBusConfig>) {
    this.identity = identity;

    // Use shared location: $HOME/.agentmux/shared/
    const sharedDir = path.join(os.homedir(), '.agentmux', 'shared');

    this.config = {
      transport: 'file',
      busPath: sharedDir,
      pollInterval: 500, // Faster polling for better responsiveness
      ...config,
    };

    if (this.config.transport !== 'file') {
      throw new Error('Only file transport is supported in MVP');
    }

    this.busPath = this.config.busPath!;
    this.messagesPath = path.join(this.busPath, 'messages');
    this.agentsPath = path.join(this.busPath, 'agents');

    this.ensureBusDirectories();
  }

  private ensureBusDirectories(): void {
    [this.busPath, this.messagesPath, this.agentsPath].forEach((dir) => {
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
    const filepath = path.join(this.messagesPath, filename);

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
      const files = fs.readdirSync(this.messagesPath);

      for (const file of files) {
        if (!file.endsWith('.json')) continue;

        const filepath = path.join(this.messagesPath, file);
        const stat = fs.statSync(filepath);

        // Only process messages newer than last read
        if (stat.mtimeMs <= this.lastReadTime) continue;

        try {
          const content = fs.readFileSync(filepath, 'utf8');
          const message: AgentMessage = JSON.parse(content);

          // Skip our own messages
          if (message.from.id === this.identity.id) continue;

          // Check if message is for us
          if (!this.isMessageForMe(message)) continue;

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

  private isMessageForMe(message: AgentMessage): boolean {
    // Broadcast
    if (message.to === '*') return true;

    // Direct ID match
    if (message.to === this.identity.id) return true;

    // Wildcard match (e.g., "Agent1-*")
    if (typeof message.to === 'string' && message.to.includes('*')) {
      const pattern = message.to.replace(/\*/g, '.*');
      return new RegExp(`^${pattern}$`).test(this.identity.id);
    }

    // Array of recipients
    if (Array.isArray(message.to)) {
      return message.to.includes(this.identity.id);
    }

    return false;
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
