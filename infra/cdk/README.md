# AgentMux Webhook Router Infrastructure

CDK infrastructure for AgentMux reactive agent communication via webhook-based shell injection.

## Overview

This infrastructure enables webhooks from external services (GitHub, CI/CD, etc.) to inject commands into specific AgentMux terminal panes in real-time.

**Architecture:**
- **HTTP API:** Receives webhooks from external services
- **WebSocket API:** Real-time delivery to AgentMux client instances
- **Lambda Function:** Routes webhooks to appropriate terminals
- **DynamoDB:** Stores webhook configurations and active connections

## Prerequisites

```bash
# Install dependencies
npm install

# Configure AWS credentials (your-aws-profile profile)
export AWS_PROFILE=your-aws-profile
export AWS_REGION=us-east-1

# Bootstrap CDK (first time only)
cdk bootstrap
```

## Deployment

```bash
# Build TypeScript
npm run build

# Synthesize CloudFormation template
npm run synth

# View changes before deployment
npm run diff

# Deploy to AWS
npm run deploy

# Destroy infrastructure (use with caution!)
npm run destroy
```

## Stack Outputs

After deployment, you'll receive:

- **HttpApiEndpoint:** `https://{api-id}.execute-api.us-east-1.amazonaws.com`
  - Use for webhook URLs: `/webhook/github`, `/webhook/custom`
  - Register subscriptions: `POST /register`
  - Health check: `GET /health`

- **WebSocketApiEndpoint:** `wss://{api-id}.execute-api.us-east-1.amazonaws.com/prod`
  - AgentMux clients connect here

- **WebhookSecretArn:** ARN for Secrets Manager secret
  - Store GitHub webhook secrets here

## Configuration

### 1. Set Webhook Secrets

```bash
# Update GitHub webhook secret
aws secretsmanager put-secret-value \
  --secret-id agentmux/webhook-secret-prod \
  --secret-string '{"github":"your-github-webhook-secret","custom":"your-custom-secret","default":"shared-secret"}'
```

### 2. Configure AgentMux Client

Add to `~/.agentmux/webhook-config.json`:

```json
{
  "version": "1.0",
  "workspaceId": "your-workspace",
  "cloudEndpoint": "wss://{api-id}.execute-api.us-east-1.amazonaws.com/prod",
  "terminals": []
}
```

Generate auth token:

```bash
# Token = HMAC-SHA256(workspaceId, secret)
python3 -c "import hmac, hashlib; print(hmac.new(b'shared-secret', b'your-workspace', hashlib.sha256).hexdigest())"
```

### 3. Register GitHub Webhook

1. Go to repository settings → Webhooks
2. Add webhook:
   - **URL:** `https://{api-id}.execute-api.us-east-1.amazonaws.com/webhook/github`
   - **Secret:** (same as in Secrets Manager)
   - **Events:** Pull requests, Issues, etc.

## Testing

### Test Health Check

```bash
curl https://{api-id}.execute-api.us-east-1.amazonaws.com/health
```

### Test Webhook Delivery

```bash
# Calculate signature
payload='{"test": "data"}'
secret="your-github-webhook-secret"
signature=$(echo -n "$payload" | openssl dgst -sha256 -hmac "$secret" | sed 's/^.* //')

# Send test webhook
curl -X POST \
  https://{api-id}.execute-api.us-east-1.amazonaws.com/webhook/github \
  -H "Content-Type: application/json" \
  -H "X-Hub-Signature-256: sha256=$signature" \
  -H "X-GitHub-Event: test" \
  -d "$payload"
```

### Test Subscription Registration

```bash
curl -X POST \
  https://{api-id}.execute-api.us-east-1.amazonaws.com/register \
  -H "Content-Type: application/json" \
  -d '{
    "workspaceId": "your-workspace",
    "terminalId": "your-workspace-term-001",
    "subscription": {
      "id": "test-sub-001",
      "provider": "github",
      "events": ["pull_request"],
      "filters": {
        "action": ["opened", "synchronize"]
      },
      "commandTemplate": "echo \"[GitHub] PR #{{pull_request.number}}: {{action}}\"\n",
      "enabled": true
    }
  }'
```

## Monitoring

View Lambda logs:

```bash
aws logs tail /aws/lambda/agentmux-webhook-router-prod --follow
```

View DynamoDB tables:

```bash
# View webhook configurations
aws dynamodb scan --table-name AgentMuxWebhookConfig-prod

# View active connections
aws dynamodb scan --table-name AgentMuxConnections-prod
```

## Cost Estimation

**Typical monthly costs (100 webhooks/day, 5 terminals):**
- API Gateway: ~$3.50
- Lambda: ~$0.20 (within free tier)
- DynamoDB: ~$1.25 (PAY_PER_REQUEST)
- Secrets Manager: ~$0.40
- **Total: ~$5.35/month**

## Security

- All webhook signatures are validated
- WebSocket connections require authentication
- Secrets stored in AWS Secrets Manager
- DynamoDB tables have encryption at rest
- CloudWatch logs for audit trail

## Troubleshooting

### Connection Issues

Check if connection is active:
```bash
aws dynamodb query \
  --table-name AgentMuxConnections-prod \
  --index-name WorkspaceIndex \
  --key-condition-expression "workspaceId = :wid" \
  --expression-attribute-values '{":wid":{"S":"your-workspace"}}'
```

### Webhook Not Delivered

1. Check Lambda logs for errors
2. Verify subscription is registered in DynamoDB
3. Confirm filters match webhook payload
4. Test WebSocket connection manually

## Related Documentation

- [SPEC_REACTIVE_AGENT_COMMUNICATION.md](../../SPEC_REACTIVE_AGENT_COMMUNICATION.md) - Full specification
- [Lambda Handler](../lambda/webhook-router/handler.py) - Python implementation
- [CDK Stack](./lib/agentmux-webhook-stack.ts) - Infrastructure definition

## Development

```bash
# Watch for changes
npm run watch

# Run tests
npm test

# Type check
npm run build
```
