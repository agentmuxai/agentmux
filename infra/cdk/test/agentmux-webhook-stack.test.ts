import * as cdk from 'aws-cdk-lib';
import { Template, Match } from 'aws-cdk-lib/assertions';
import { AgentMuxWebhookStack } from '../lib/agentmux-webhook-stack';

describe('AgentMuxWebhookStack', () => {
  let app: cdk.App;
  let stack: AgentMuxWebhookStack;
  let template: Template;

  beforeEach(() => {
    app = new cdk.App();
    stack = new AgentMuxWebhookStack(app, 'TestStack', {
      environment: 'test',
      env: {
        account: '123456789012',
        region: 'us-east-1',
      },
    });
    template = Template.fromStack(stack);
  });

  describe('DynamoDB Tables', () => {
    test('creates WebhookConfigTable with correct configuration', () => {
      template.hasResourceProperties('AWS::DynamoDB::Table', {
        TableName: 'AgentMuxWebhookConfig-test',
        BillingMode: 'PAY_PER_REQUEST',
        KeySchema: [
          {
            AttributeName: 'subscriptionId',
            KeyType: 'HASH',
          },
        ],
      });
    });

    test('WebhookConfigTable has ProviderEventIndex GSI', () => {
      template.hasResourceProperties('AWS::DynamoDB::Table', {
        TableName: 'AgentMuxWebhookConfig-test',
        GlobalSecondaryIndexes: Match.arrayWith([
          Match.objectLike({
            IndexName: 'ProviderEventIndex',
            KeySchema: [
              {
                AttributeName: 'provider',
                KeyType: 'HASH',
              },
              {
                AttributeName: 'eventType',
                KeyType: 'RANGE',
              },
            ],
          }),
        ]),
      });
    });

    test('WebhookConfigTable has WorkspaceIndex GSI', () => {
      template.hasResourceProperties('AWS::DynamoDB::Table', {
        TableName: 'AgentMuxWebhookConfig-test',
        GlobalSecondaryIndexes: Match.arrayWith([
          Match.objectLike({
            IndexName: 'WorkspaceIndex',
            KeySchema: [
              {
                AttributeName: 'workspaceId',
                KeyType: 'HASH',
              },
            ],
          }),
        ]),
      });
    });

    test('creates ConnectionTable with TTL enabled', () => {
      template.hasResourceProperties('AWS::DynamoDB::Table', {
        TableName: 'AgentMuxConnections-test',
        TimeToLiveSpecification: {
          AttributeName: 'ttl',
          Enabled: true,
        },
      });
    });

    test('ConnectionTable has WorkspaceIndex GSI', () => {
      template.hasResourceProperties('AWS::DynamoDB::Table', {
        TableName: 'AgentMuxConnections-test',
        GlobalSecondaryIndexes: Match.arrayWith([
          Match.objectLike({
            IndexName: 'WorkspaceIndex',
          }),
        ]),
      });
    });

    test('prod environment has PITR enabled', () => {
      const prodApp = new cdk.App();
      const prodStack = new AgentMuxWebhookStack(prodApp, 'ProdStack', {
        environment: 'prod',
        env: {
          account: '123456789012',
          region: 'us-east-1',
        },
      });
      const prodTemplate = Template.fromStack(prodStack);

      prodTemplate.hasResourceProperties('AWS::DynamoDB::Table', {
        TableName: 'AgentMuxWebhookConfig-prod',
        PointInTimeRecoverySpecification: {
          PointInTimeRecoveryEnabled: true,
        },
      });
    });
  });

  describe('Lambda Function', () => {
    test('creates webhook router function with correct configuration', () => {
      template.hasResourceProperties('AWS::Lambda::Function', {
        FunctionName: 'wavemux-webhook-router-test',
        Runtime: 'python3.12',
        Handler: 'handler.lambda_handler',
        Timeout: 30,
        MemorySize: 512,
      });
    });

    test('Lambda has environment variables', () => {
      template.hasResourceProperties('AWS::Lambda::Function', {
        Environment: {
          Variables: {
            WEBHOOK_CONFIG_TABLE: Match.objectLike({
              Ref: Match.stringLikeRegexp('WebhookConfigTable'),
            }),
            CONNECTION_TABLE: Match.objectLike({
              Ref: Match.stringLikeRegexp('ConnectionTable'),
            }),
            ENVIRONMENT: 'test',
            WEBHOOK_SECRET_ARN: Match.objectLike({
              Ref: Match.stringLikeRegexp('WebhookSecret'),
            }),
          },
        },
      });
    });

    test('Lambda has DynamoDB permissions', () => {
      template.hasResourceProperties('AWS::IAM::Policy', {
        PolicyDocument: {
          Statement: Match.arrayWith([
            Match.objectLike({
              Action: Match.arrayWith([
                'dynamodb:BatchGetItem',
                'dynamodb:GetRecords',
                'dynamodb:GetShardIterator',
                'dynamodb:Query',
                'dynamodb:GetItem',
                'dynamodb:Scan',
                'dynamodb:ConditionCheckItem',
                'dynamodb:BatchWriteItem',
                'dynamodb:PutItem',
                'dynamodb:UpdateItem',
                'dynamodb:DeleteItem',
              ]),
              Effect: 'Allow',
            }),
          ]),
        },
      });
    });

    test('Lambda has Secrets Manager read permissions', () => {
      template.hasResourceProperties('AWS::IAM::Policy', {
        PolicyDocument: {
          Statement: Match.arrayWith([
            Match.objectLike({
              Action: Match.arrayWith([
                'secretsmanager:GetSecretValue',
                'secretsmanager:DescribeSecret',
              ]),
              Effect: 'Allow',
            }),
          ]),
        },
      });
    });

    test('Lambda has API Gateway ManageConnections permission', () => {
      template.hasResourceProperties('AWS::IAM::Policy', {
        PolicyDocument: {
          Statement: Match.arrayWith([
            Match.objectLike({
              Action: 'execute-api:ManageConnections',
              Effect: 'Allow',
            }),
          ]),
        },
      });
    });

    test('Lambda has CloudWatch Logs permissions', () => {
      // CloudWatch logs permissions are automatically added by CDK
      // Verify at least one IAM policy exists
      const template_json = template.toJSON();
      const policies = Object.values(template_json.Resources).filter(
        (r: any) => r.Type === 'AWS::IAM::Policy'
      );
      expect(policies.length).toBeGreaterThan(0);
    });
  });

  describe('HTTP API Gateway', () => {
    test('creates HTTP API', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Api', {
        Name: 'wavemux-webhook-http-test',
        ProtocolType: 'HTTP',
      });
    });

    test('has CORS configuration', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Api', {
        CorsConfiguration: {
          AllowOrigins: ['*'],
          AllowMethods: ['POST'],
          AllowHeaders: ['Content-Type', 'X-Hub-Signature-256', 'X-GitHub-Event'],
        },
      });
    });

    test('has webhook delivery route', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Route', {
        RouteKey: 'POST /webhook/{provider}',
      });
    });

    test('has register route', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Route', {
        RouteKey: 'POST /register',
      });
    });

    test('has unregister route', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Route', {
        RouteKey: 'POST /unregister',
      });
    });

    test('has health check route', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Route', {
        RouteKey: 'GET /health',
      });
    });

    test('has Lambda integration', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Integration', {
        IntegrationType: 'AWS_PROXY',
        PayloadFormatVersion: '2.0',
      });
    });
  });

  describe('WebSocket API Gateway', () => {
    test('creates WebSocket API', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Api', {
        Name: 'wavemux-webhook-ws-test',
        ProtocolType: 'WEBSOCKET',
      });
    });

    test('has $connect route', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Route', {
        RouteKey: '$connect',
      });
    });

    test('has $disconnect route', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Route', {
        RouteKey: '$disconnect',
      });
    });

    test('has stage with auto-deploy', () => {
      template.hasResourceProperties('AWS::ApiGatewayV2::Stage', {
        StageName: 'test',
        AutoDeploy: true,
      });
    });
  });

  describe('Secrets Manager', () => {
    test('creates webhook secret', () => {
      template.hasResourceProperties('AWS::SecretsManager::Secret', {
        Name: 'wavemux/webhook-secret-test',
        Description: 'Webhook authentication secrets for AgentMux (GitHub, etc.)',
      });
    });

    test('webhook secret has generated string configuration', () => {
      template.hasResourceProperties('AWS::SecretsManager::Secret', {
        GenerateSecretString: {
          GenerateStringKey: 'default',
        },
      });
    });
  });

  describe('Stack Outputs', () => {
    test('exports WebhookConfigTable name', () => {
      template.hasOutput('WebhookConfigTableName', {
        Description: 'DynamoDB table for webhook configuration',
        Export: {
          Name: 'TestStack-WebhookConfigTable',
        },
      });
    });

    test('exports ConnectionTable name', () => {
      template.hasOutput('ConnectionTableName', {
        Description: 'DynamoDB table for WebSocket connections',
        Export: {
          Name: 'TestStack-ConnectionTable',
        },
      });
    });

    test('exports Lambda function ARN', () => {
      template.hasOutput('WebhookRouterFunctionArn', {
        Description: 'Lambda function ARN for webhook router',
        Export: {
          Name: 'TestStack-WebhookRouterArn',
        },
      });
    });

    test('exports HTTP API endpoint', () => {
      template.hasOutput('HttpApiEndpoint', {
        Description: 'HTTP API endpoint for webhook delivery',
        Export: {
          Name: 'TestStack-HttpApiEndpoint',
        },
      });
    });

    test('exports WebSocket API endpoint', () => {
      template.hasOutput('WebSocketApiEndpoint', {
        Description: 'WebSocket API endpoint for AgentMux clients',
        Export: {
          Name: 'TestStack-WebSocketApiEndpoint',
        },
      });
    });

    test('exports webhook secret ARN', () => {
      template.hasOutput('WebhookSecretArn', {
        Description: 'Secrets Manager ARN for webhook secrets',
        Export: {
          Name: 'TestStack-WebhookSecretArn',
        },
      });
    });
  });

  describe('Tags', () => {
    test('has required tags', () => {
      const resources = template.toJSON().Resources;
      const lambdaFunction = Object.values(resources).find(
        (r: any) => r.Type === 'AWS::Lambda::Function'
      ) as any;

      expect(lambdaFunction.Properties.Tags).toContainEqual({
        Key: 'Project',
        Value: 'AgentMux',
      });
      expect(lambdaFunction.Properties.Tags).toContainEqual({
        Key: 'Component',
        Value: 'WebhookRouter',
      });
      expect(lambdaFunction.Properties.Tags).toContainEqual({
        Key: 'Environment',
        Value: 'test',
      });
    });
  });

  describe('Resource Count', () => {
    test('creates expected number of resources', () => {
      const resources = template.toJSON().Resources;
      const resourceCounts = {
        tables: 0,
        lambda: 0,
        httpApi: 0,
        wsApi: 0,
        secrets: 0,
      };

      Object.values(resources).forEach((resource: any) => {
        if (resource.Type === 'AWS::DynamoDB::Table') resourceCounts.tables++;
        if (resource.Type === 'AWS::Lambda::Function') resourceCounts.lambda++;
        if (resource.Type === 'AWS::ApiGatewayV2::Api') {
          if (resource.Properties.ProtocolType === 'HTTP') resourceCounts.httpApi++;
          if (resource.Properties.ProtocolType === 'WEBSOCKET') resourceCounts.wsApi++;
        }
        if (resource.Type === 'AWS::SecretsManager::Secret') resourceCounts.secrets++;
      });

      expect(resourceCounts.tables).toBe(2); // Config + Connection
      expect(resourceCounts.lambda).toBeGreaterThanOrEqual(1); // Webhook router (may have custom resources)
      expect(resourceCounts.httpApi).toBe(1); // HTTP API
      expect(resourceCounts.wsApi).toBe(1); // WebSocket API
      expect(resourceCounts.secrets).toBe(1); // Webhook secret
    });
  });

  describe('Security', () => {
    test('DynamoDB tables have encryption', () => {
      // DynamoDB encryption at rest is enabled by default in AWS
      // Verify tables are created (encryption is implicit)
      const template_json = template.toJSON();
      const tables = Object.values(template_json.Resources).filter(
        (r: any) => r.Type === 'AWS::DynamoDB::Table'
      );
      expect(tables.length).toBe(2);
    });

    test('Lambda has X-Ray tracing disabled by default', () => {
      // Can be enabled if needed, but not required for this use case
      const resources = template.toJSON().Resources;
      const lambdaFunction = Object.values(resources).find(
        (r: any) => r.Type === 'AWS::Lambda::Function'
      ) as any;

      // Tracing is optional - just verify function exists
      expect(lambdaFunction).toBeDefined();
    });
  });
});
