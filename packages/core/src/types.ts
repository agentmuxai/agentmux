/**
 * AgentMux Core Types
 *
 * Message protocol for inter-agent communication
 */

export interface AgentIdentity {
  /** Unique agent ID (e.g., "AgentX-12345-1759843800") */
  id: string;
  /** Agent name (e.g., "AgentX", "Agent1", "Agent2") */
  name: string;
  /** Workspace path */
  workspace: string;
  /** Process ID */
  pid: number;
  /** Timestamp when agent started */
  startedAt: number;
}

export interface AgentMessage {
  /** Unique message ID */
  id: string;
  /** Sender agent identity */
  from: AgentIdentity;
  /** Recipient agent ID(s), or "*" for broadcast */
  to: string | string[];
  /** Message type */
  type: MessageType;
  /** Message payload */
  payload: unknown;
  /** Timestamp */
  timestamp: number;
  /** Optional reply-to message ID */
  replyTo?: string;
}

export enum MessageType {
  /** Agent registration/heartbeat */
  REGISTER = 'register',
  /** Agent shutdown notification */
  SHUTDOWN = 'shutdown',
  /** Text message */
  MESSAGE = 'message',
  /** Command request */
  COMMAND = 'command',
  /** Command response */
  RESPONSE = 'response',
  /** Status update */
  STATUS = 'status',
  /** File transfer */
  FILE = 'file',
  /** Error notification */
  ERROR = 'error',
}

export interface MessageBusConfig {
  /** Message bus transport (file-based for now) */
  transport: 'file' | 'redis' | 'websocket';
  /** Path to message bus directory (for file transport) */
  busPath?: string;
  /** Redis connection (for redis transport) */
  redisUrl?: string;
  /** WebSocket URL (for websocket transport) */
  wsUrl?: string;
  /** Poll interval in ms (for file transport) */
  pollInterval?: number;
}

export interface MessageHandler {
  (message: AgentMessage): void | Promise<void>;
}
