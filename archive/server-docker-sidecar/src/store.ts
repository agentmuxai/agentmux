import { DynamoDBClient } from "@aws-sdk/client-dynamodb";
import {
  DynamoDBDocumentClient,
  PutCommand,
  QueryCommand,
  BatchWriteCommand,
  ScanCommand,
  DeleteCommand,
  GetCommand,
} from "@aws-sdk/lib-dynamodb";
import { randomUUID } from "crypto";

export interface Message {
  id: string;
  from_agent: string;
  to_agent: string;
  text: string;
  priority: "low" | "normal" | "high" | "urgent";
  timestamp: string;
  read: boolean;
}

export interface Agent {
  id: string;
  last_seen: string;
  messages_sent: number;
}

export interface Injection {
  id: string;
  target_agent: string;
  source_agent: string;
  message: string;
  priority: "normal" | "urgent";
  status: "pending" | "delivered" | "expired";
  created_at: string;
  delivered_at?: string;
  ttl: number; // DynamoDB TTL (epoch seconds)
}

/** Interface for store operations (for testing/mocking) */
export interface IMessageStore {
  sendMessage(from: string, to: string, text: string, priority?: string): Promise<Message>;
  readMessages(agentId: string, unreadOnly?: boolean, limit?: number, markAsRead?: boolean): Promise<Message[]>;
  listAgents(): Promise<Agent[]>;
  deleteMessages(agentId: string, messageIds: string[]): Promise<{ deleted: string[]; errors: { id: string; error: string }[] }>;
  getStats(): Promise<{ total_messages: number; unread_messages: number; unique_agents: number }>;
  // Reactive injection methods
  createInjection(sourceAgent: string, targetAgent: string, message: string, priority?: "normal" | "urgent"): Promise<Injection>;
  getPendingInjections(targetAgent: string): Promise<Injection[]>;
  acknowledgeInjections(agentId: string, injectionIds: string[]): Promise<{ acknowledged: string[]; errors: { id: string; error: string }[] }>;
}

export class MessageStore implements IMessageStore {
  private client: DynamoDBDocumentClient;
  private messagesTable: string;
  private agentsTable: string;
  private injectionsTable: string;

  constructor() {
    const dynamoClient = new DynamoDBClient({});
    this.client = DynamoDBDocumentClient.from(dynamoClient);
    this.messagesTable = process.env.MESSAGES_TABLE_NAME || 'agentmux-messages-prod';
    this.agentsTable = process.env.AGENTS_TABLE_NAME || 'agentmux-agents-prod';
    this.injectionsTable = process.env.INJECTIONS_TABLE_NAME || 'agentmux-injections-prod';
  }

  async sendMessage(from: string, to: string, text: string, priority: string = "normal"): Promise<Message> {
    const id = `msg-${Date.now()}-${randomUUID().slice(0, 8)}`;
    const timestamp = new Date().toISOString();
    const message: Message = {
      id,
      from_agent: from,
      to_agent: to,
      text,
      priority: priority as Message["priority"],
      timestamp,
      read: false
    };

    // Put message in messages table
    await this.client.send(new PutCommand({
      TableName: this.messagesTable,
      Item: message
    }));

    // Update agent last_seen and message count
    await this.updateAgent(from);

    return message;
  }

  async readMessages(agentId: string, unreadOnly: boolean = true, limit: number = 10, markAsRead: boolean = true): Promise<Message[]> {
    // Helper function to paginate queries until we get enough unread messages
    const paginateQuery = async (baseParams: any, targetLimit: number): Promise<Message[]> => {
      const collected: Message[] = [];
      let lastEvaluatedKey: any = undefined;

      // Keep querying until we have enough unread messages or exhaust the index
      while (collected.length < targetLimit) {
        const params = { ...baseParams };
        if (lastEvaluatedKey) {
          params.ExclusiveStartKey = lastEvaluatedKey;
        }

        const result = await this.client.send(new QueryCommand(params));
        const items = (result.Items || []) as Message[];

        collected.push(...items);

        // If no more results, break
        if (!result.LastEvaluatedKey) {
          break;
        }

        lastEvaluatedKey = result.LastEvaluatedKey;

        // If we have enough unread messages, break
        if (collected.length >= targetLimit) {
          break;
        }
      }

      return collected;
    };

    // Query messages for this agent using GSI
    const queryParams: any = {
      TableName: this.messagesTable,
      IndexName: 'to_agent-timestamp-index',
      KeyConditionExpression: 'to_agent = :agent',
      ExpressionAttributeValues: {
        ':agent': agentId
      },
      ScanIndexForward: false // DESC order (newest first)
    };

    // Add filter for unread messages
    if (unreadOnly) {
      queryParams.FilterExpression = '#read = :false';
      queryParams.ExpressionAttributeNames = { '#read': 'read' };
      queryParams.ExpressionAttributeValues[':false'] = false;
    }

    // Paginate to get up to limit unread messages
    const messages = await paginateQuery(queryParams, limit);

    // Also query for broadcast messages (to_agent = '*')
    const broadcastParams: any = {
      TableName: this.messagesTable,
      IndexName: 'to_agent-timestamp-index',
      KeyConditionExpression: 'to_agent = :broadcast',
      ExpressionAttributeValues: {
        ':broadcast': '*'
      },
      ScanIndexForward: false
    };

    if (unreadOnly) {
      broadcastParams.FilterExpression = '#read = :false';
      broadcastParams.ExpressionAttributeNames = { '#read': 'read' };
      broadcastParams.ExpressionAttributeValues[':false'] = false;
    }

    // Paginate broadcast messages too
    const broadcastMessages = await paginateQuery(broadcastParams, limit);

    // Combine and sort by timestamp
    const allMessages = [...messages, ...broadcastMessages]
      .sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())
      .slice(0, limit);

    // Mark messages as read
    if (markAsRead && allMessages.length > 0) {
      // Update each message individually (DynamoDB doesn't support bulk updates easily)
      await Promise.all(allMessages.map(msg =>
        this.client.send(new PutCommand({
          TableName: this.messagesTable,
          Item: { ...msg, read: true }
        }))
      ));

      // Return updated messages
      return allMessages.map(m => ({ ...m, read: true }));
    }

    return allMessages;
  }

  async listAgents(): Promise<Agent[]> {
    // Scan agents table
    const result = await this.client.send(new ScanCommand({
      TableName: this.agentsTable
    }));

    const agents = (result.Items || []) as Agent[];

    // Sort by last_seen DESC
    return agents.sort((a, b) =>
      new Date(b.last_seen).getTime() - new Date(a.last_seen).getTime()
    );
  }

  async deleteMessages(agentId: string, messageIds: string[]): Promise<{ deleted: string[]; errors: { id: string; error: string }[] }> {
    const deleted: string[] = [];
    const errors: { id: string; error: string }[] = [];

    for (const id of messageIds) {
      try {
        // First, get the message to check authorization
        const getResult = await this.client.send(new QueryCommand({
          TableName: this.messagesTable,
          KeyConditionExpression: 'id = :id',
          ExpressionAttributeValues: { ':id': id }
        }));

        if (!getResult.Items || getResult.Items.length === 0) {
          errors.push({ id, error: "Message not found" });
          continue;
        }

        const msg = getResult.Items[0] as Message;

        // Check authorization
        if (msg.to_agent !== agentId && msg.from_agent !== agentId && msg.to_agent !== "*") {
          errors.push({ id, error: "Not authorized" });
          continue;
        }

        // Delete the message
        await this.client.send(new DeleteCommand({
          TableName: this.messagesTable,
          Key: { id }
        }));

        deleted.push(id);
      } catch (err) {
        errors.push({ id, error: (err as Error).message });
      }
    }

    return { deleted, errors };
  }

  async getStats() {
    // Helper function to paginate scan and accumulate counts
    const paginateScan = async (scanParams: any): Promise<number> => {
      let totalCount = 0;
      let lastEvaluatedKey: any = undefined;

      do {
        const params = { ...scanParams };
        if (lastEvaluatedKey) {
          params.ExclusiveStartKey = lastEvaluatedKey;
        }

        const result = await this.client.send(new ScanCommand(params));
        totalCount += result.Count || 0;
        lastEvaluatedKey = result.LastEvaluatedKey;
      } while (lastEvaluatedKey);

      return totalCount;
    };

    // Scan messages table for total count
    const total_messages = await paginateScan({
      TableName: this.messagesTable,
      Select: 'COUNT'
    });

    // Scan for unread messages count
    const unread_messages = await paginateScan({
      TableName: this.messagesTable,
      FilterExpression: '#read = :false',
      ExpressionAttributeNames: { '#read': 'read' },
      ExpressionAttributeValues: { ':false': false },
      Select: 'COUNT'
    });

    // Scan agents table for count
    const unique_agents = await paginateScan({
      TableName: this.agentsTable,
      Select: 'COUNT'
    });

    return {
      total_messages,
      unread_messages,
      unique_agents
    };
  }

  async cleanup(maxAgeHours: number = 24): Promise<number> {
    const cutoff = new Date(Date.now() - maxAgeHours * 60 * 60 * 1000).toISOString();

    // Scan for old messages
    const result = await this.client.send(new ScanCommand({
      TableName: this.messagesTable,
      FilterExpression: '#timestamp < :cutoff',
      ExpressionAttributeNames: { '#timestamp': 'timestamp' },
      ExpressionAttributeValues: { ':cutoff': cutoff }
    }));

    const oldMessages = result.Items || [];

    // Delete old messages in batches of 25 (DynamoDB limit)
    let deleted = 0;
    for (let i = 0; i < oldMessages.length; i += 25) {
      const batch = oldMessages.slice(i, i + 25);
      await this.client.send(new BatchWriteCommand({
        RequestItems: {
          [this.messagesTable]: batch.map(item => ({
            DeleteRequest: { Key: { id: item.id } }
          }))
        }
      }));
      deleted += batch.length;
    }

    return deleted;
  }

  private async updateAgent(agentId: string) {
    // Get current agent or create new
    const getResult = await this.client.send(new QueryCommand({
      TableName: this.agentsTable,
      KeyConditionExpression: 'id = :id',
      ExpressionAttributeValues: { ':id': agentId }
    }));

    const currentAgent = getResult.Items?.[0] as Agent | undefined;
    const messages_sent = (currentAgent?.messages_sent || 0) + 1;

    await this.client.send(new PutCommand({
      TableName: this.agentsTable,
      Item: {
        id: agentId,
        last_seen: new Date().toISOString(),
        messages_sent
      }
    }));
  }

  // =============================================
  // Reactive Injection Methods
  // =============================================

  async createInjection(sourceAgent: string, targetAgent: string, message: string, priority: "normal" | "urgent" = "normal"): Promise<Injection> {
    const id = `inj-${Date.now()}-${randomUUID().slice(0, 8)}`;
    const created_at = new Date().toISOString();
    // TTL: 1 hour from now (epoch seconds)
    const ttl = Math.floor(Date.now() / 1000) + 3600;

    const injection: Injection = {
      id,
      target_agent: targetAgent,
      source_agent: sourceAgent,
      message,
      priority,
      status: "pending",
      created_at,
      ttl
    };

    await this.client.send(new PutCommand({
      TableName: this.injectionsTable,
      Item: injection
    }));

    return injection;
  }

  async getPendingInjections(targetAgent: string): Promise<Injection[]> {
    // Query using GSI: target_agent-created_at-index
    const result = await this.client.send(new QueryCommand({
      TableName: this.injectionsTable,
      IndexName: 'target_agent-created_at-index',
      KeyConditionExpression: 'target_agent = :agent',
      FilterExpression: '#status = :pending',
      ExpressionAttributeNames: { '#status': 'status' },
      ExpressionAttributeValues: {
        ':agent': targetAgent,
        ':pending': 'pending'
      },
      ScanIndexForward: true // ASC order (oldest first for FIFO processing)
    }));

    return (result.Items || []) as Injection[];
  }

  async acknowledgeInjections(agentId: string, injectionIds: string[]): Promise<{ acknowledged: string[]; errors: { id: string; error: string }[] }> {
    const acknowledged: string[] = [];
    const errors: { id: string; error: string }[] = [];
    const delivered_at = new Date().toISOString();

    for (const id of injectionIds) {
      try {
        // Get injection first to verify it exists (use GetCommand for partition key lookup)
        const getResult = await this.client.send(new GetCommand({
          TableName: this.injectionsTable,
          Key: { id }
        }));

        if (!getResult.Item) {
          errors.push({ id, error: "Injection not found" });
          continue;
        }

        const injection = getResult.Item as Injection;

        // Security: Verify caller is the target agent
        if (injection.target_agent !== agentId) {
          errors.push({ id, error: "Not authorized - not target agent" });
          continue;
        }

        // Update status to delivered
        await this.client.send(new PutCommand({
          TableName: this.injectionsTable,
          Item: {
            ...injection,
            status: "delivered",
            delivered_at
          }
        }));

        acknowledged.push(id);
      } catch (err) {
        errors.push({ id, error: (err as Error).message });
      }
    }

    return { acknowledged, errors };
  }
}
