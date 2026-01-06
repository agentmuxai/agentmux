import { DynamoDBClient } from '@aws-sdk/client-dynamodb';
import {
  DynamoDBDocumentClient,
  PutCommand,
  QueryCommand,
  UpdateCommand,
  DeleteCommand,
} from '@aws-sdk/lib-dynamodb';
import { AgentMessage, Agent } from './types';

const client = new DynamoDBClient({ region: process.env.AWS_REGION || 'us-east-1' });
const docClient = DynamoDBDocumentClient.from(client);

/**
 * Store a message in DynamoDB
 */
export async function saveMessage(
  message: AgentMessage,
  tableName: string
): Promise<void> {
  await docClient.send(
    new PutCommand({
      TableName: tableName,
      Item: message,
    })
  );
}

/**
 * Get messages for a recipient
 */
export async function getMessages(
  recipientId: string,
  tableName: string,
  limit: number = 100
): Promise<AgentMessage[]> {
  const result = await docClient.send(
    new QueryCommand({
      TableName: tableName,
      KeyConditionExpression: 'recipientId = :recipientId',
      ExpressionAttributeValues: {
        ':recipientId': recipientId,
      },
      Limit: limit,
      ScanIndexForward: false, // Most recent first
    })
  );

  return (result.Items as AgentMessage[]) || [];
}

/**
 * Delete messages by IDs
 */
export async function deleteMessages(
  recipientId: string,
  messageIds: string[],
  tableName: string
): Promise<void> {
  // Note: DynamoDB doesn't support batch delete by message ID directly
  // In production, you'd query by recipientId+timestamp and then batch delete
  // For simplicity, this is a placeholder
  for (const messageId of messageIds) {
    // Would need to fetch timestamp first, skipping for now
    console.warn('Delete by messageId not fully implemented');
  }
}

/**
 * Update agent status
 */
export async function updateAgentStatus(
  agentId: string,
  status: 'online' | 'offline',
  tableName: string,
  metadata?: Record<string, any>
): Promise<void> {
  await docClient.send(
    new UpdateCommand({
      TableName: tableName,
      Key: { agentId },
      UpdateExpression:
        'SET #status = :status, lastSeen = :lastSeen' +
        (metadata ? ', metadata = :metadata' : ''),
      ExpressionAttributeNames: {
        '#status': 'status',
      },
      ExpressionAttributeValues: {
        ':status': status,
        ':lastSeen': Date.now(),
        ...(metadata && { ':metadata': metadata }),
      },
    })
  );
}

/**
 * Get all agents
 */
export async function getAllAgents(tableName: string): Promise<Agent[]> {
  const result = await docClient.send(
    new QueryCommand({
      TableName: tableName,
      IndexName: 'status-lastSeen-index',
      KeyConditionExpression: '#status = :status',
      ExpressionAttributeNames: {
        '#status': 'status',
      },
      ExpressionAttributeValues: {
        ':status': 'online',
      },
    })
  );

  return (result.Items as Agent[]) || [];
}
