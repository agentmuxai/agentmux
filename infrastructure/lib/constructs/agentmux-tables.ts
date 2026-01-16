import * as cdk from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import { Construct } from 'constructs';

export interface AgentMuxTablesProps {
  /**
   * Environment name (e.g., 'prod', 'dev')
   */
  environment?: string;
}

/**
 * DynamoDB tables for AgentMux message persistence and agent registry.
 */
export class AgentMuxTables extends Construct {
  public readonly messagesTable: dynamodb.Table;
  public readonly agentsTable: dynamodb.Table;
  public readonly injectionsTable: dynamodb.Table;

  constructor(scope: Construct, id: string, props?: AgentMuxTablesProps) {
    super(scope, id);

    const env = props?.environment || 'prod';

    // Messages table: stores agent-to-agent messages
    this.messagesTable = new dynamodb.Table(this, 'MessagesTable', {
      tableName: `agentmux-messages-${env}`,
      partitionKey: { name: 'id', type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      timeToLiveAttribute: 'ttl', // Optional: auto-cleanup old messages
      pointInTimeRecoverySpecification: {
        pointInTimeRecoveryEnabled: true,
      },
      removalPolicy: cdk.RemovalPolicy.RETAIN,
    });

    // GSI: Query messages by recipient (to_agent) and timestamp
    this.messagesTable.addGlobalSecondaryIndex({
      indexName: 'to_agent-timestamp-index',
      partitionKey: { name: 'to_agent', type: dynamodb.AttributeType.STRING },
      sortKey: { name: 'timestamp', type: dynamodb.AttributeType.STRING },
      projectionType: dynamodb.ProjectionType.ALL,
    });

    // Agents table: stores agent registry and presence
    this.agentsTable = new dynamodb.Table(this, 'AgentsTable', {
      tableName: `agentmux-agents-${env}`,
      partitionKey: { name: 'id', type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      pointInTimeRecoverySpecification: {
        pointInTimeRecoveryEnabled: true,
      },
      removalPolicy: cdk.RemovalPolicy.RETAIN,
    });

    // Injections table: stores cross-host reactive terminal injections
    this.injectionsTable = new dynamodb.Table(this, 'InjectionsTable', {
      tableName: `agentmux-injections-${env}`,
      partitionKey: { name: 'id', type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      timeToLiveAttribute: 'ttl', // Auto-cleanup after 1 hour
      pointInTimeRecoverySpecification: {
        pointInTimeRecoveryEnabled: true,
      },
      removalPolicy: cdk.RemovalPolicy.RETAIN,
    });

    // GSI: Query injections by target agent and creation time
    this.injectionsTable.addGlobalSecondaryIndex({
      indexName: 'target_agent-created_at-index',
      partitionKey: { name: 'target_agent', type: dynamodb.AttributeType.STRING },
      sortKey: { name: 'created_at', type: dynamodb.AttributeType.STRING },
      projectionType: dynamodb.ProjectionType.ALL,
    });

    // Tags
    cdk.Tags.of(this.messagesTable).add('Component', 'agentmux');
    cdk.Tags.of(this.agentsTable).add('Component', 'agentmux');
    cdk.Tags.of(this.injectionsTable).add('Component', 'agentmux');
  }
}
