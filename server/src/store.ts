import { DynamoDBClient } from "@aws-sdk/client-dynamodb";
import {
  DynamoDBDocumentClient,
  PutCommand,
  QueryCommand,
  BatchWriteCommand,
  ScanCommand,
  DeleteCommand,
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

export class MessageStore {
  private client: DynamoDBDocumentClient;
  private messagesTable: string;
  private agentsTable: string;

  constructor() {
    const dynamoClient = new DynamoDBClient({});
    this.client = DynamoDBDocumentClient.from(dynamoClient);
    this.messagesTable = process.env.MESSAGES_TABLE_NAME || 'agentmux-messages-prod';
    this.agentsTable = process.env.AGENTS_TABLE_NAME || 'agentmux-agents-prod';
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
    // Query messages for this agent using GSI
    const queryParams: any = {
      TableName: this.messagesTable,
      IndexName: 'to_agent-timestamp-index',
      KeyConditionExpression: 'to_agent = :agent',
      ExpressionAttributeValues: {
        ':agent': agentId
      },
      Limit: limit,
      ScanIndexForward: false // DESC order (newest first)
    };

    // Add filter for unread messages
    if (unreadOnly) {
      queryParams.FilterExpression = '#read = :false';
      queryParams.ExpressionAttributeNames = { '#read': 'read' };
      queryParams.ExpressionAttributeValues[':false'] = false;
    }

    const result = await this.client.send(new QueryCommand(queryParams));
    const messages = (result.Items || []) as Message[];

    // Also query for broadcast messages (to_agent = '*')
    const broadcastParams: any = {
      TableName: this.messagesTable,
      IndexName: 'to_agent-timestamp-index',
      KeyConditionExpression: 'to_agent = :broadcast',
      ExpressionAttributeValues: {
        ':broadcast': '*'
      },
      Limit: limit,
      ScanIndexForward: false
    };

    if (unreadOnly) {
      broadcastParams.FilterExpression = '#read = :false';
      broadcastParams.ExpressionAttributeNames = { '#read': 'read' };
      broadcastParams.ExpressionAttributeValues[':false'] = false;
    }

    const broadcastResult = await this.client.send(new QueryCommand(broadcastParams));
    const broadcastMessages = (broadcastResult.Items || []) as Message[];

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
    // Scan messages table for stats
    const messagesResult = await this.client.send(new ScanCommand({
      TableName: this.messagesTable,
      Select: 'COUNT'
    }));

    const unreadResult = await this.client.send(new ScanCommand({
      TableName: this.messagesTable,
      FilterExpression: '#read = :false',
      ExpressionAttributeNames: { '#read': 'read' },
      ExpressionAttributeValues: { ':false': false },
      Select: 'COUNT'
    }));

    const agentsResult = await this.client.send(new ScanCommand({
      TableName: this.agentsTable,
      Select: 'COUNT'
    }));

    return {
      total_messages: messagesResult.Count || 0,
      unread_messages: unreadResult.Count || 0,
      unique_agents: agentsResult.Count || 0
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
}
