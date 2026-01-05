import * as cdk from 'aws-cdk-lib';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as logs from 'aws-cdk-lib/aws-logs';
import { Construct } from 'constructs';
import { NodejsFunction } from 'aws-cdk-lib/aws-lambda-nodejs';
import { AgentMuxTables } from './constructs/agentmux-tables';
import * as path from 'path';

export interface AgentMuxStackProps extends cdk.StackProps {
  /**
   * Environment name (e.g., 'prod', 'dev')
   */
  environment?: string;
}

/**
 * AgentMux infrastructure stack.
 *
 * Imports bastion resources via CloudFormation exports and:
 * - Creates DynamoDB tables for messages and agents
 * - Adds port 8443 ingress to bastion security group
 * - Grants DynamoDB permissions to bastion IAM role
 */
export class AgentMuxStack extends cdk.Stack {
  public readonly tables: AgentMuxTables;

  constructor(scope: Construct, id: string, props: AgentMuxStackProps) {
    super(scope, id, props);

    const env = props.environment || 'prod';

    // ----------------------------------------
    // Import Bastion Resources
    // ----------------------------------------
    const bastionInstanceId = cdk.Fn.importValue('infrastructure-bastion-instance-id');
    const bastionSgId = cdk.Fn.importValue('infrastructure-bastion-sg-id');
    const bastionRoleArn = cdk.Fn.importValue('infrastructure-bastion-role-arn');

    // Reference imported resources
    const bastionSG = ec2.SecurityGroup.fromSecurityGroupId(
      this,
      'BastionSecurityGroup',
      bastionSgId
    );

    const bastionRole = iam.Role.fromRoleArn(
      this,
      'BastionRole',
      bastionRoleArn,
      { mutable: true } // Allow modifications
    );

    // ----------------------------------------
    // DynamoDB Tables
    // ----------------------------------------
    this.tables = new AgentMuxTables(this, 'Tables', { environment: env });

    // ----------------------------------------
    // Lambda Function
    // ----------------------------------------
    const agentmuxFunction = new NodejsFunction(this, 'AgentMuxFunction', {
      runtime: lambda.Runtime.NODEJS_20_X,
      handler: 'handler',
      entry: path.join(__dirname, '../../server/src/lambda.ts'),
      functionName: 'agentmux-server',
      timeout: cdk.Duration.seconds(30),
      memorySize: 512,
      environment: {
        MESSAGES_TABLE_NAME: this.tables.messagesTable.tableName,
        AGENTS_TABLE_NAME: this.tables.agentsTable.tableName,
        NODE_ENV: 'production'
      },
      bundling: {
        minify: true,
        sourceMap: true,
        externalModules: [],
        format: lambda.OutputFormat.ESM,
        mainFields: ['module', 'main'],
        banner: "import { createRequire } from 'module';const require = createRequire(import.meta.url);"
      },
      logRetention: logs.RetentionDays.ONE_WEEK
    });

    // Grant DynamoDB permissions
    this.tables.messagesTable.grantReadWriteData(agentmuxFunction);
    this.tables.agentsTable.grantReadWriteData(agentmuxFunction);

    // Function URL (public endpoint)
    const functionUrl = agentmuxFunction.addFunctionUrl({
      authType: lambda.FunctionUrlAuthType.NONE,
      cors: {
        allowedOrigins: ['*'],
        allowedMethods: [lambda.HttpMethod.ALL],
        allowedHeaders: ['*']
      }
    });

    // ----------------------------------------
    // Security Group: Allow WebSocket connections
    // ----------------------------------------
    bastionSG.addIngressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(8443),
      'AgentMux WebSocket server (TLS)'
    );

    // ----------------------------------------
    // IAM: Grant DynamoDB permissions to bastion
    // ----------------------------------------
    this.tables.messagesTable.grantReadWriteData(bastionRole);
    this.tables.agentsTable.grantReadWriteData(bastionRole);

    // Grant Secrets Manager access for JWT secret
    new iam.Policy(this, 'BastionSecretsPolicy', {
      roles: [bastionRole],
      statements: [
        new iam.PolicyStatement({
          effect: iam.Effect.ALLOW,
          actions: ['secretsmanager:GetSecretValue'],
          resources: [`arn:aws:secretsmanager:${this.region}:${this.account}:secret:services/infra-*`],
        }),
      ],
    });

    // ----------------------------------------
    // Outputs
    // ----------------------------------------
    new cdk.CfnOutput(this, 'AgentMuxUrl', {
      value: functionUrl.url,
      description: 'AgentMux Lambda Function URL',
      exportName: `agentmux-url-${env}`,
    });

    new cdk.CfnOutput(this, 'AgentMuxFunctionName', {
      value: agentmuxFunction.functionName,
      description: 'AgentMux Lambda function name',
    });

    new cdk.CfnOutput(this, 'MessagesTableName', {
      value: this.tables.messagesTable.tableName,
      description: 'DynamoDB table for agent messages',
      exportName: `agentmux-messages-table-${env}`,
    });

    new cdk.CfnOutput(this, 'AgentsTableName', {
      value: this.tables.agentsTable.tableName,
      description: 'DynamoDB table for agent registry',
      exportName: `agentmux-agents-table-${env}`,
    });

    new cdk.CfnOutput(this, 'BastionInstanceId', {
      value: bastionInstanceId,
      description: 'Bastion EC2 instance ID (imported)',
    });

    new cdk.CfnOutput(this, 'BastionSecurityGroupId', {
      value: bastionSgId,
      description: 'Bastion security group ID (imported)',
    });

    // Apply stack tags
    cdk.Tags.of(this).add('Project', 'agentmux');
    cdk.Tags.of(this).add('Component', 'infrastructure');
    cdk.Tags.of(this).add('ManagedBy', 'CDK');
  }
}
