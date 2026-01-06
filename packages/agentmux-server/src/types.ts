import { WebSocket } from 'ws';

/**
 * Message sent between agents
 */
export interface AgentMessage {
  messageId: string;
  senderId: string;
  recipientId: string;
  message: string;
  priority: 'low' | 'normal' | 'high' | 'urgent';
  timestamp: number;
  ttl: number; // Unix timestamp for DynamoDB TTL
}

/**
 * Agent registration info
 */
export interface Agent {
  agentId: string;
  status: 'online' | 'offline';
  lastSeen: number;
  metadata?: Record<string, any>;
}

/**
 * WebSocket client with agent metadata
 */
export interface AuthenticatedClient {
  ws: WebSocket;
  agentId: string;
  connectedAt: number;
}

/**
 * Environment configuration
 */
export interface Config {
  port: number;
  jwtSecret: string;
  messagesTableName: string;
  agentsTableName: string;
  region: string;
  secretName: string;
}
