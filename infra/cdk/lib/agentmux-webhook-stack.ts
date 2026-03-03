import * as cdk from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as apigatewayv2 from 'aws-cdk-lib/aws-apigatewayv2';
import * as apigatewayv2Integrations from 'aws-cdk-lib/aws-apigatewayv2-integrations';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import { Construct } from 'constructs';

interface AgentMuxWebhookStackProps extends cdk.StackProps {
  environment: string;
}

export class AgentMuxWebhookStack extends cdk.Stack {
  public readonly webhookConfigTable: dynamodb.Table;
  public readonly connectionTable: dynamodb.Table;
  public readonly webhookRouterFunction: lambda.Function;
  public readonly httpApi: apigatewayv2.HttpApi;
  public readonly webSocketApi: apigatewayv2.WebSocketApi;

  constructor(scope: Construct, id: string, props: AgentMuxWebhookStackProps) {
    super(scope, id, props);

    const { environment } = props;

    // ==========================================================================
    // DynamoDB Tables
    // ==========================================================================

    // Table for webhook configuration and subscriptions
    this.webhookConfigTable = new dynamodb.Table(this, 'WebhookConfigTable', {
      tableName: `AgentMuxWebhookConfig-${environment}`,
      partitionKey: {
        name: 'subscriptionId',
        type: dynamodb.AttributeType.STRING,
      },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      removalPolicy: environment === 'prod' ? cdk.RemovalPolicy.RETAIN : cdk.RemovalPolicy.DESTROY,
      pointInTimeRecovery: environment === 'prod',
    });

    // Global Secondary Index for querying by provider and event type
    this.webhookConfigTable.addGlobalSecondaryIndex({
      indexName: 'ProviderEventIndex',
      partitionKey: {
        name: 'provider',
        type: dynamodb.AttributeType.STRING,
      },
      sortKey: {
        name: 'eventType',
        type: dynamodb.AttributeType.STRING,
      },
      projectionType: dynamodb.ProjectionType.ALL,
    });

    // GSI for querying by workspace
    this.webhookConfigTable.addGlobalSecondaryIndex({
      indexName: 'WorkspaceIndex',
      partitionKey: {
        name: 'workspaceId',
        type: dynamodb.AttributeType.STRING,
      },
      projectionType: dynamodb.ProjectionType.ALL,
    });

    // Table for active WebSocket connections
    this.connectionTable = new dynamodb.Table(this, 'ConnectionTable', {
      tableName: `AgentMuxConnections-${environment}`,
      partitionKey: {
        name: 'connectionId',
        type: dynamodb.AttributeType.STRING,
      },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      timeToLiveAttribute: 'ttl',
      removalPolicy: cdk.RemovalPolicy.DESTROY, // Always destroy - ephemeral data
    });

    // GSI for querying connections by workspace
    this.connectionTable.addGlobalSecondaryIndex({
      indexName: 'WorkspaceIndex',
      partitionKey: {
        name: 'workspaceId',
        type: dynamodb.AttributeType.STRING,
      },
      projectionType: dynamodb.ProjectionType.ALL,
    });

    // ==========================================================================
    // Secrets Manager
    // ==========================================================================

    // Use existing services/prod secret instead of creating a new one
    // Webhook secrets will be stored at: services/prod["agentmux"]["GITHUB_WEBHOOK_SECRET"]
    const servicesSecret = secretsmanager.Secret.fromSecretNameV2(
      this,
      'ServicesSecret',
      'services/prod'
    );

    // ==========================================================================
    // Lambda Function - Webhook Router
    // ==========================================================================

    this.webhookRouterFunction = new lambda.Function(this, 'WebhookRouterFunction', {
      functionName: `agentmux-webhook-router-${environment}`,
      runtime: lambda.Runtime.PYTHON_3_12,
      handler: 'handler.lambda_handler',
      code: lambda.Code.fromAsset('../lambda/webhook-router'),
      timeout: cdk.Duration.seconds(30),
      memorySize: 512,
      environment: {
        WEBHOOK_CONFIG_TABLE: this.webhookConfigTable.tableName,
        CONNECTION_TABLE: this.connectionTable.tableName,
        ENVIRONMENT: environment,
        SECRET_NAME: 'services/prod',
        PROJECT_NAME: 'agentmux',
      },
      logRetention: logs.RetentionDays.ONE_WEEK,
    });

    // Grant permissions
    this.webhookConfigTable.grantReadWriteData(this.webhookRouterFunction);
    this.connectionTable.grantReadWriteData(this.webhookRouterFunction);
    servicesSecret.grantRead(this.webhookRouterFunction);

    // ==========================================================================
    // WebSocket API - For AgentMux Client Connections
    // ==========================================================================

    this.webSocketApi = new apigatewayv2.WebSocketApi(this, 'WebSocketApi', {
      apiName: `agentmux-webhook-ws-${environment}`,
      description: 'WebSocket API for AgentMux webhook delivery',
      connectRouteOptions: {
        integration: new apigatewayv2Integrations.WebSocketLambdaIntegration(
          'ConnectIntegration',
          this.webhookRouterFunction
        ),
      },
      disconnectRouteOptions: {
        integration: new apigatewayv2Integrations.WebSocketLambdaIntegration(
          'DisconnectIntegration',
          this.webhookRouterFunction
        ),
      },
    });

    const webSocketStage = new apigatewayv2.WebSocketStage(this, 'WebSocketStage', {
      webSocketApi: this.webSocketApi,
      stageName: environment,
      autoDeploy: true,
    });

    // Grant Lambda permission to manage WebSocket connections
    this.webhookRouterFunction.addToRolePolicy(
      new iam.PolicyStatement({
        actions: ['execute-api:ManageConnections'],
        resources: [
          `arn:aws:execute-api:${this.region}:${this.account}:${this.webSocketApi.apiId}/${environment}/*`,
        ],
      })
    );

    // ==========================================================================
    // HTTP API - For Webhook Delivery
    // ==========================================================================

    this.httpApi = new apigatewayv2.HttpApi(this, 'HttpApi', {
      apiName: `agentmux-webhook-http-${environment}`,
      description: 'HTTP API for receiving webhooks from external services',
      corsPreflight: {
        allowOrigins: ['*'],
        allowMethods: [apigatewayv2.CorsHttpMethod.POST],
        allowHeaders: ['Content-Type', 'X-Hub-Signature-256', 'X-GitHub-Event'],
      },
    });

    // Route: POST /webhook/{provider}
    this.httpApi.addRoutes({
      path: '/webhook/{provider}',
      methods: [apigatewayv2.HttpMethod.POST],
      integration: new apigatewayv2Integrations.HttpLambdaIntegration(
        'WebhookDeliveryIntegration',
        this.webhookRouterFunction
      ),
    });

    // Route: POST /register
    this.httpApi.addRoutes({
      path: '/register',
      methods: [apigatewayv2.HttpMethod.POST],
      integration: new apigatewayv2Integrations.HttpLambdaIntegration(
        'RegisterIntegration',
        this.webhookRouterFunction
      ),
    });

    // Route: POST /unregister
    this.httpApi.addRoutes({
      path: '/unregister',
      methods: [apigatewayv2.HttpMethod.POST],
      integration: new apigatewayv2Integrations.HttpLambdaIntegration(
        'UnregisterIntegration',
        this.webhookRouterFunction
      ),
    });

    // Route: GET /health
    this.httpApi.addRoutes({
      path: '/health',
      methods: [apigatewayv2.HttpMethod.GET],
      integration: new apigatewayv2Integrations.HttpLambdaIntegration(
        'HealthIntegration',
        this.webhookRouterFunction
      ),
    });

    // ==========================================================================
    // Outputs
    // ==========================================================================

    new cdk.CfnOutput(this, 'WebhookConfigTableName', {
      value: this.webhookConfigTable.tableName,
      description: 'DynamoDB table for webhook configuration',
      exportName: `${id}-WebhookConfigTable`,
    });

    new cdk.CfnOutput(this, 'ConnectionTableName', {
      value: this.connectionTable.tableName,
      description: 'DynamoDB table for WebSocket connections',
      exportName: `${id}-ConnectionTable`,
    });

    new cdk.CfnOutput(this, 'WebhookRouterFunctionArn', {
      value: this.webhookRouterFunction.functionArn,
      description: 'Lambda function ARN for webhook router',
      exportName: `${id}-WebhookRouterArn`,
    });

    new cdk.CfnOutput(this, 'HttpApiEndpoint', {
      value: this.httpApi.apiEndpoint,
      description: 'HTTP API endpoint for webhook delivery',
      exportName: `${id}-HttpApiEndpoint`,
    });

    new cdk.CfnOutput(this, 'WebSocketApiEndpoint', {
      value: webSocketStage.url,
      description: 'WebSocket API endpoint for AgentMux clients',
      exportName: `${id}-WebSocketApiEndpoint`,
    });

    new cdk.CfnOutput(this, 'SecretName', {
      value: 'services/prod',
      description: 'Secrets Manager secret name (project: agentmux)',
      exportName: `${id}-SecretName`,
    });

    // ==========================================================================
    // Tags
    // ==========================================================================

    cdk.Tags.of(this).add('Project', 'AgentMux');
    cdk.Tags.of(this).add('Component', 'WebhookRouter');
    cdk.Tags.of(this).add('Environment', environment);
  }
}
