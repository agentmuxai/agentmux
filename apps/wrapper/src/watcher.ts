import * as chokidar from 'chokidar';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { Message } from './types';

export interface WatcherOptions {
  agentId: string;
  messagesDir?: string;
  onMessage: (message: Message) => void;
  debug?: boolean;
}

/**
 * Watches the message bus for new messages addressed to this agent
 */
export class MessageWatcher {
  private agentId: string;
  private messagesDir: string;
  private onMessage: (message: Message) => void;
  private watcher: chokidar.FSWatcher | null = null;
  private processedMessages: Set<string> = new Set();
  private debug: boolean;

  constructor(options: WatcherOptions) {
    this.agentId = options.agentId;
    this.messagesDir = options.messagesDir || path.join(os.homedir(), '.agentmux/shared/messages');
    this.onMessage = options.onMessage;
    this.debug = options.debug || false;

    this.log('MessageWatcher initialized', { agentId: this.agentId, messagesDir: this.messagesDir });
  }

  async start(): Promise<void> {
    // Ensure messages directory exists
    if (!fs.existsSync(this.messagesDir)) {
      this.log('Creating messages directory', { path: this.messagesDir });
      fs.mkdirSync(this.messagesDir, { recursive: true });
    }

    // Start watching for new JSON files
    this.watcher = chokidar.watch(`${this.messagesDir}/*.json`, {
      ignoreInitial: true,
      awaitWriteFinish: {
        stabilityThreshold: 100,
        pollInterval: 50
      }
    });

    this.watcher.on('add', (filePath) => this.handleNewFile(filePath));

    this.log('Watcher started');
  }

  private async handleNewFile(filePath: string): Promise<void> {
    const messageId = path.basename(filePath, '.json');

    // Skip if already processed
    if (this.processedMessages.has(messageId)) {
      this.log('Message already processed', { messageId });
      return;
    }

    try {
      // Read and parse message
      const content = fs.readFileSync(filePath, 'utf8');
      const message: Message = JSON.parse(content);

      this.log('New message detected', { id: message.id, from: message.from.name, to: message.to });

      // Check if message is for this agent
      if (this.isForMe(message)) {
        this.log('Message is for this agent', { id: message.id });
        this.processedMessages.add(messageId);
        this.onMessage(message);
      } else {
        this.log('Message is not for this agent', { id: message.id, expectedAgentId: this.agentId });
      }
    } catch (error) {
      console.error('[MessageWatcher] Error processing message:', filePath, error);
    }
  }

  private isForMe(message: Message): boolean {
    // Check if message is addressed to this agent
    // Supports:
    // - Direct: "AgentX"
    // - Pattern: "AgentX-*"
    // - Broadcast: "*"

    if (message.to === '*') {
      return true; // Broadcast to all
    }

    if (message.to === this.agentId) {
      return true; // Direct match
    }

    // Pattern matching (e.g., "AgentX-*" matches "AgentX")
    if (message.to.endsWith('-*')) {
      const prefix = message.to.slice(0, -2);
      if (this.agentId.startsWith(prefix)) {
        return true;
      }
    }

    return false;
  }

  stop(): void {
    this.log('Stopping watcher');

    if (this.watcher) {
      this.watcher.close();
      this.watcher = null;
    }

    this.processedMessages.clear();
  }

  async stopAsync(): Promise<void> {
    this.log('Stopping watcher (async)');

    if (this.watcher) {
      await this.watcher.close();
      this.watcher = null;
    }

    this.processedMessages.clear();
  }

  private log(message: string, data?: any): void {
    if (!this.debug) return;

    const timestamp = new Date().toISOString();
    console.error(`[${timestamp}] [MessageWatcher] ${message}`, data ? JSON.stringify(data) : '');
  }
}
