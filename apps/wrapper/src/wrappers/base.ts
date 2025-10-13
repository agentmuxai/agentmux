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
  protected cliCommand?: string;  // Override command (e.g., 'claude.exe' for WSL)

  abstract get command(): string;

  /**
   * Get command-line arguments (override in subclasses)
   */
  protected getArgs(): string[] {
    return [];
  }

  constructor(options: WrapperOptions = {}) {
    this.agentId = options.agentId || process.env.AGENT_ID || 'AgentX';
    this.debug = options.debug || false;
    this.cliCommand = options.cliCommand;  // Store override command

    // Initialize message watcher
    this.watcher = new MessageWatcher({
      agentId: this.agentId,
      messagesDir: options.messagesDir,
      onMessage: (message) => this.handleMessage(message),
      debug: this.debug
    });

    this.log('Initialized BaseWrapper', { agentId: this.agentId, cliCommand: this.cliCommand });
  }

  async start(): Promise<void> {
    this.log('Starting CLI wrapper');

    if (!pty) {
      // Fallback mode: No PTY, only message watching
      console.log('\x1b[33m⚠️  Running in fallback mode (node-pty not available)\x1b[0m');
      console.log('\x1b[90m    Message notifications will appear in console only\x1b[0m');
      console.log('\x1b[90m    No automatic command injection\x1b[0m');
      console.log('\x1b[90m    Use MCP tools to check messages manually\x1b[0m\n');

      // Start watcher only (no PTY)
      await this.watcher.start();
      this.log('Message watcher started (fallback mode)');

      console.log('\x1b[32m✓ Wrapper ready - monitoring messages\x1b[0m');
      console.log('\x1b[90m  Press Ctrl+C to stop\x1b[0m\n');
      return;
    }

    // Spawn CLI in PTY
    // Use override command if provided (for WSL .exe suffix), otherwise use default
    const commandToSpawn = this.cliCommand || this.command;
    const args = this.getArgs();
    this.log('Spawning CLI', { command: commandToSpawn, args });

    this.ptyProcess = pty.spawn(commandToSpawn, args, {
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
    // ANSI color codes
    const BG_BLUE = '\x1b[44m';
    const BG_RED = '\x1b[41m';
    const BOLD = '\x1b[1m';
    const RESET = '\x1b[0m';

    // Use red for urgent messages
    const bgColor = message.priority === 'urgent' ? BG_RED : BG_BLUE;
    const icon = message.priority === 'urgent' ? '⚠️' : '📨';

    const notification = `${bgColor}${BOLD} ${icon}  Remote message from ${message.from.name} ${RESET}`;

    if (this.ptyProcess) {
      // PTY mode: Inject into terminal
      this.ptyProcess.write('\n');
      this.ptyProcess.write(notification + '\n');
    } else {
      // Fallback mode: Print to console
      console.log('\n' + notification);
      console.log(`\x1b[90m    From: ${message.from.name}\x1b[0m`);
      console.log(`\x1b[90m    Use MCP tools to read: agentmux_list_messages\x1b[0m\n`);
    }
  }

  inject(command: string): void {
    if (!this.ptyProcess) {
      // Fallback mode: Can't inject commands, just log
      this.log('Skipping command injection (fallback mode)', { command });
      return;
    }

    this.log('Injecting command', { command });
    this.ptyProcess.write(`${command}\n`);
  }

  stop(): void {
    this.log('Stopping wrapper');

    // Restore terminal FIRST to prevent freeze
    if (process.stdin.setRawMode) {
      try {
        process.stdin.setRawMode(false);
        this.log('Terminal raw mode disabled');
      } catch (error) {
        this.log('Error disabling raw mode', { error });
      }
    }

    // Remove all stdin listeners to prevent hanging
    process.stdin.removeAllListeners('data');
    process.stdin.pause();

    // Stop message watcher
    if (this.watcher) {
      this.watcher.stop();
      this.log('Watcher stopped');
    }

    // Kill PTY process
    if (this.ptyProcess) {
      try {
        this.ptyProcess.kill('SIGTERM');
        this.log('PTY process terminated');
      } catch (error) {
        this.log('Error killing PTY', { error });
      }
      this.ptyProcess = null;
    }

    this.log('Wrapper stopped - terminal restored');
  }

  protected log(message: string, data?: any): void {
    if (!this.debug) return;

    const timestamp = new Date().toISOString();
    console.error(`[${timestamp}] [BaseWrapper] ${message}`, data ? JSON.stringify(data) : '');
  }
}
