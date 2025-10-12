import { AIWrapper, Message, WrapperOptions } from '../types';
import { MessageWatcher } from '../watcher';

// Try to import node-pty, but make it optional
let pty: any = null;
try {
  pty = require('node-pty');
} catch (error) {
  console.warn('[BaseWrapper] node-pty not available, some features may be limited');
}

/**
 * Base wrapper class providing common functionality for all AI CLI wrappers
 */
export abstract class BaseWrapper implements AIWrapper {
  protected ptyProcess: any | null = null;
  protected agentId: string;
  protected watcher: MessageWatcher;
  protected debug: boolean;

  abstract get command(): string;

  constructor(options: WrapperOptions = {}) {
    this.agentId = options.agentId || process.env.AGENT_ID || 'AgentX';
    this.debug = options.debug || false;

    // Initialize message watcher
    this.watcher = new MessageWatcher({
      agentId: this.agentId,
      messagesDir: options.messagesDir,
      onMessage: (message) => this.handleMessage(message),
      debug: this.debug
    });

    this.log('Initialized BaseWrapper', { agentId: this.agentId });
  }

  async start(): Promise<void> {
    this.log('Starting CLI wrapper');

    if (!pty) {
      throw new Error(
        'node-pty is required but not installed. ' +
        'Please install it with: npm install node-pty\n' +
        'Note: On Windows, you may need Visual Studio build tools.'
      );
    }

    // Spawn CLI in PTY
    this.ptyProcess = pty.spawn(this.command, [], {
      name: 'xterm-color',
      cols: process.stdout.columns || 80,
      rows: process.stdout.rows || 30,
      cwd: process.cwd(),
      env: process.env
    });

    this.log('PTY process spawned', { pid: this.ptyProcess.pid });

    // Set up I/O proxying
    this.setupIO();

    // Start watching for messages
    await this.watcher.start();

    this.log('Message watcher started');
  }

  protected setupIO(): void {
    if (!this.ptyProcess) {
      throw new Error('PTY process not initialized');
    }

    // Proxy PTY output to stdout (human sees everything)
    this.ptyProcess.onData((data: string) => {
      process.stdout.write(data);
    });

    // Proxy stdin to PTY (human can type)
    process.stdin.setRawMode?.(true);
    process.stdin.on('data', (data: Buffer) => {
      if (this.ptyProcess) {
        this.ptyProcess.write(data.toString());
      }
    });

    // Handle resize
    process.stdout.on('resize', () => {
      if (this.ptyProcess) {
        this.ptyProcess.resize(
          process.stdout.columns || 80,
          process.stdout.rows || 30
        );
      }
    });
  }

  protected handleMessage(message: Message): void {
    this.log('Handling remote message', { from: message.from.name, id: message.id });

    // Show highlighted notification
    this.showNotification(message);

    // Inject command to check messages
    this.inject('check messages');
  }

  protected showNotification(message: Message): void {
    if (!this.ptyProcess) return;

    // ANSI color codes
    const BG_BLUE = '\x1b[44m';
    const BG_RED = '\x1b[41m';
    const BOLD = '\x1b[1m';
    const RESET = '\x1b[0m';

    // Use red for urgent messages
    const bgColor = message.priority === 'urgent' ? BG_RED : BG_BLUE;
    const icon = message.priority === 'urgent' ? '⚠️' : '📨';

    // Write highlighted notification
    this.ptyProcess.write('\n');
    this.ptyProcess.write(
      `${bgColor}${BOLD} ${icon}  Remote message from ${message.from.name} ${RESET}\n`
    );
  }

  inject(command: string): void {
    if (!this.ptyProcess) {
      throw new Error('PTY process not initialized');
    }

    this.log('Injecting command', { command });
    this.ptyProcess.write(`${command}\n`);
  }

  stop(): void {
    this.log('Stopping wrapper');

    if (this.watcher) {
      this.watcher.stop();
    }

    if (this.ptyProcess) {
      this.ptyProcess.kill();
      this.ptyProcess = null;
    }

    // Restore terminal
    if (process.stdin.setRawMode) {
      process.stdin.setRawMode(false);
    }

    this.log('Wrapper stopped');
  }

  protected log(message: string, data?: any): void {
    if (!this.debug) return;

    const timestamp = new Date().toISOString();
    console.error(`[${timestamp}] [BaseWrapper] ${message}`, data ? JSON.stringify(data) : '');
  }
}
