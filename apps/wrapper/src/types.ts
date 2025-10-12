/**
 * Shared types for AgentMux wrapper
 */

export interface Message {
  id: string;
  from: {
    id: string;
    name: string;
  };
  to: string;
  payload: {
    text: string;
    [key: string]: any;
  };
  timestamp: string;
  priority?: 'normal' | 'urgent';
}

export interface Agent {
  id: string;
  name: string;
  status: 'active' | 'idle';
  lastSeen: string;
}

export interface WrapperOptions {
  agentId?: string;
  messagesDir?: string;
  debug?: boolean;
}

export interface AIWrapper {
  /**
   * The CLI command to wrap (e.g., 'claude', 'gemini')
   */
  readonly command: string;

  /**
   * Start the wrapped CLI process
   */
  start(): Promise<void>;

  /**
   * Inject a command into the running CLI
   */
  inject(command: string): void;

  /**
   * Stop the wrapped CLI process
   */
  stop(): void;
}
