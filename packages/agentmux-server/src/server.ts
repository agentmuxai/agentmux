import { WebSocketServer, WebSocket } from 'ws';
import { v4 as uuidv4 } from 'uuid';
import { verifyToken } from './auth.js';
import {
  saveMessage,
  getMessages,
  updateAgentStatus,
  getAllAgents,
  deleteMessages,
} from './db.js';
import { AuthenticatedClient, AgentMessage, Config } from './types.js';

export class AgentMuxServer {
  private wss: WebSocketServer;
  private clients: Map<string, AuthenticatedClient> = new Map();
  private config: Config;

  constructor(config: Config) {
    this.config = config;
    this.wss = new WebSocketServer({ port: config.port });

    this.wss.on('connection', this.handleConnection.bind(this));
    console.log(`AgentMux server listening on port ${config.port}`);
  }

  private handleConnection(ws: WebSocket, request: any) {
    // Extract token from query string or headers
    const url = new URL(request.url, `ws://localhost:${this.config.port}`);
    const token = url.searchParams.get('token') || request.headers['authorization']?.replace('Bearer ', '');

    if (!token) {
      ws.close(4001, 'No authentication token provided');
      return;
    }

    const agentId = verifyToken(token, this.config.jwtSecret);
    if (!agentId) {
      ws.close(4002, 'Invalid authentication token');
      return;
    }

    // Register client
    const client: AuthenticatedClient = {
      ws,
      agentId,
      connectedAt: Date.now(),
    };
    this.clients.set(agentId, client);

    console.log(`Agent connected: ${agentId}`);

    // Update agent status to online
    updateAgentStatus(agentId, 'online', this.config.agentsTableName).catch((err) =>
      console.error('Failed to update agent status:', err)
    );

    // Handle messages from client
    ws.on('message', (data) => this.handleMessage(client, data));

    // Handle disconnect
    ws.on('close', () => this.handleDisconnect(client));

    // Send welcome message
    this.sendToClient(client, {
      type: 'welcome',
      agentId,
      timestamp: Date.now(),
    });
  }

  private async handleMessage(client: AuthenticatedClient, data: any) {
    try {
      const message = JSON.parse(data.toString());

      switch (message.type) {
        case 'send_message':
          await this.handleSendMessage(client, message);
          break;
        case 'read_messages':
          await this.handleReadMessages(client, message);
          break;
        case 'list_agents':
          await this.handleListAgents(client);
          break;
        case 'broadcast_message':
          await this.handleBroadcast(client, message);
          break;
        case 'delete_messages':
          await this.handleDeleteMessages(client, message);
          break;
        case 'ping':
          this.sendToClient(client, { type: 'pong', timestamp: Date.now() });
          break;
        default:
          this.sendToClient(client, {
            type: 'error',
            error: `Unknown message type: ${message.type}`,
          });
      }
    } catch (error) {
      console.error('Error handling message:', error);
      this.sendToClient(client, {
        type: 'error',
        error: String(error),
      });
    }
  }

  private async handleSendMessage(client: AuthenticatedClient, request: any) {
    const { to, message, priority = 'normal' } = request;

    const agentMessage: AgentMessage = {
      messageId: uuidv4(),
      senderId: client.agentId,
      recipientId: to,
      message,
      priority,
      timestamp: Date.now(),
      ttl: Math.floor(Date.now() / 1000) + 7 * 24 * 60 * 60, // 7 days
    };

    // Save to DynamoDB
    await saveMessage(agentMessage, this.config.messagesTableName);

    // Try to deliver immediately if recipient is online
    const recipient = this.clients.get(to);
    if (recipient) {
      this.sendToClient(recipient, {
        type: 'message',
        ...agentMessage,
      });
    }

    // Confirm to sender
    this.sendToClient(client, {
      type: 'message_sent',
      messageId: agentMessage.messageId,
      timestamp: agentMessage.timestamp,
    });
  }

  private async handleReadMessages(client: AuthenticatedClient, request: any) {
    const { unread_only = true, limit = 100 } = request;

    const messages = await getMessages(
      client.agentId,
      this.config.messagesTableName,
      limit
    );

    this.sendToClient(client, {
      type: 'messages',
      messages,
      count: messages.length,
    });
  }

  private async handleListAgents(client: AuthenticatedClient) {
    const agents = await getAllAgents(this.config.agentsTableName);

    this.sendToClient(client, {
      type: 'agents',
      agents: agents.map((a) => ({
        agentId: a.agentId,
        status: a.status,
        lastSeen: a.lastSeen,
      })),
    });
  }

  private async handleBroadcast(client: AuthenticatedClient, request: any) {
    const { message, priority = 'normal' } = request;

    // Send to all connected agents except sender
    const timestamp = Date.now();
    const ttl = Math.floor(timestamp / 1000) + 7 * 24 * 60 * 60;

    for (const [agentId, recipient] of this.clients.entries()) {
      if (agentId === client.agentId) continue;

      const agentMessage: AgentMessage = {
        messageId: uuidv4(),
        senderId: client.agentId,
        recipientId: agentId,
        message,
        priority,
        timestamp,
        ttl,
      };

      // Save to DynamoDB
      await saveMessage(agentMessage, this.config.messagesTableName);

      // Deliver immediately
      this.sendToClient(recipient, {
        type: 'message',
        ...agentMessage,
      });
    }

    this.sendToClient(client, {
      type: 'broadcast_sent',
      recipients: this.clients.size - 1,
      timestamp,
    });
  }

  private async handleDeleteMessages(client: AuthenticatedClient, request: any) {
    const { message_ids = [] } = request;

    await deleteMessages(
      client.agentId,
      message_ids,
      this.config.messagesTableName
    );

    this.sendToClient(client, {
      type: 'messages_deleted',
      count: message_ids.length,
    });
  }

  private handleDisconnect(client: AuthenticatedClient) {
    console.log(`Agent disconnected: ${client.agentId}`);
    this.clients.delete(client.agentId);

    // Update agent status to offline
    updateAgentStatus(client.agentId, 'offline', this.config.agentsTableName).catch(
      (err) => console.error('Failed to update agent status:', err)
    );
  }

  private sendToClient(client: AuthenticatedClient, data: any) {
    if (client.ws.readyState === WebSocket.OPEN) {
      client.ws.send(JSON.stringify(data));
    }
  }

  public close() {
    this.wss.close();
  }
}
