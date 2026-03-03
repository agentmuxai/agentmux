# AgentMux Webhook Infrastructure

**Status:** 🚀 Deployed and Operational (October 29, 2025)

Independent AWS CDK infrastructure for AgentMux webhook-based reactive agent communication.

## Overview

This infrastructure enables external webhooks (from GitHub, CI/CD, monitoring systems, etc.) to inject commands into specific AgentMux terminal panes in real-time.

**Key Features:**
- ✅ Real-time webhook → terminal command injection
- ✅ WebSocket-based delivery for low latency
- ✅ Multi-provider support (GitHub, custom webhooks)
- ✅ Flexible command templates with data transformation
- ✅ Workspace isolation and authentication
- ✅ Serverless architecture (Lambda + API Gateway + DynamoDB)

## API Endpoints

After deployment, your stack outputs will contain:

- **HTTP API (Webhook Ingestion):** `https://{api-id}.execute-api.{region}.amazonaws.com`
- **WebSocket API (Real-time Delivery):** `wss://{ws-api-id}.execute-api.{region}.amazonaws.com/prod`

### Health Check
```bash
curl https://{api-id}.execute-api.{region}.amazonaws.com/health
```

## Architecture

```
GitHub/CI/CD Webhook
    │
    ├─> API Gateway (HTTP)
    │       ├─> Lambda (webhook-router)
    │       │       ├─> DynamoDB (subscriptions)
    │       │       └─> API Gateway (WebSocket)
    │       │               └─> AgentMux Client
    │       │                       └─> Terminal PTY
```

**Components:**

1. **HTTP API Gateway:** Receives webhooks from external services
2. **WebSocket API Gateway:** Real-time connection to AgentMux instances
3. **Lambda Function:** Routes webhooks and transforms payloads
4. **DynamoDB Tables:**
   - `AgentMuxWebhookConfig-{env}` - Webhook subscriptions
   - `AgentMuxConnections-{env}` - Active WebSocket connections
5. **Secrets Manager:** Stores webhook authentication secrets

## Directory Structure

```
infra/
├── cdk/                          # CDK infrastructure code
│   ├── bin/cdk.ts               # CDK app entry point
│   ├── lib/
│   │   └── agentmux-webhook-stack.ts  # Main stack definition
│   ├── package.json             # Dependencies
│   ├── tsconfig.json            # TypeScript config
│   └── README.md                # CDK-specific docs
├── lambda/                       # Lambda function code
│   └── webhook-router/
│       ├── handler.py           # Main webhook router logic
│       └── requirements.txt     # Python dependencies
├── DEPLOYMENT.md                 # Deployment guide
└── README.md                     # This file
```

## Deployment

### Prerequisites
- AWS CLI configured with your AWS profile
- Node.js 18+
- AWS CDK CLI (`npm install -g aws-cdk`)

### Deploy

```bash
cd cdk
npm install
npm run build
cdk deploy --profile your-aws-profile
```

### Outputs

After deployment, you'll get:
- **HttpApiEndpoint:** For webhooks and registration
- **WebSocketApiEndpoint:** For AgentMux client connections
- **WebhookSecretArn:** For storing webhook secrets

## Configuration

### 1. Set Webhook Secrets

```bash
aws secretsmanager put-secret-value \
  --secret-id agentmux/webhook-secret-prod \
  --secret-string '{
    "github": "your-github-webhook-secret",
    "custom": "your-custom-secret",
    "default": "shared-secret-for-auth"
  }'
```

### 2. Configure GitHub Webhook

Add webhook in repository settings:
- **URL:** `https://{api-id}.execute-api.us-east-1.amazonaws.com/webhook/github`
- **Secret:** (from Secrets Manager)
- **Events:** Pull requests, Issues, etc.

### 3. Configure AgentMux Client

Create `~/.agentmux/webhook-config.json`:

```json
{
  "version": "1.0",
  "workspaceId": "your-workspace",
  "authToken": "generated-hmac-token",
  "cloudEndpoint": "wss://{api-id}.execute-api.us-east-1.amazonaws.com/prod",
  "terminals": []
}
```

## Usage Examples

### Register Terminal Subscription

```bash
curl -X POST https://{api-id}.execute-api.us-east-1.amazonaws.com/register \
  -H "Content-Type: application/json" \
  -d '{
    "workspaceId": "your-workspace",
    "terminalId": "your-workspace-term-001",
    "subscription": {
      "id": "github-pr-monitor",
      "provider": "github",
      "events": ["pull_request"],
      "filters": {
        "action": ["opened", "synchronize", "closed"],
        "repository.name": "agentmux"
      },
      "commandTemplate": "echo \"[GitHub] PR #{{pull_request.number}}: {{action}}\"\n",
      "enabled": true
    }
  }'
```

### Command Templates

Templates use `{{path.to.value}}` syntax to extract data from webhooks:

**GitHub PR opened:**
```bash
"echo \"[PR] #{{pull_request.number}}: {{pull_request.title}} by @{{pull_request.user.login}}\"\n"
```

**CI/CD workflow completed:**
```bash
"echo \"[Actions] {{workflow.name}}: {{workflow_run.conclusion}}\"\n"
```

**Custom notification:**
```bash
"notify-send \"Alert\" \"{{message}}\" && echo \"{{timestamp}}: {{message}}\"\n"
```

## Monitoring

### View Lambda Logs

```bash
aws logs tail /aws/lambda/agentmux-webhook-router-prod --follow
```

### Check Active Connections

```bash
aws dynamodb scan --table-name AgentMuxConnections-prod
```

### View Webhook Subscriptions

```bash
aws dynamodb scan --table-name AgentMuxWebhookConfig-prod
```

## Cost Estimation

**Typical monthly costs (100 webhooks/day):**
- API Gateway: ~$3.50
- Lambda: ~$0.20 (within free tier)
- DynamoDB: ~$1.25
- Secrets Manager: ~$0.40
- **Total: ~$5.35/month**

## Security

- ✅ Webhook signature validation (HMAC-SHA256)
- ✅ WebSocket authentication tokens
- ✅ Workspace isolation
- ✅ Secrets in AWS Secrets Manager
- ✅ DynamoDB encryption at rest
- ✅ CloudWatch audit logging
- ✅ IAM least-privilege policies

## Testing

### Health Check

```bash
curl https://{api-id}.execute-api.us-east-1.amazonaws.com/health
```

### Test Webhook

```bash
# Calculate GitHub signature
payload='{"test": true}'
secret="your-github-secret"
signature=$(echo -n "$payload" | openssl dgst -sha256 -hmac "$secret" | sed 's/^.* //')

# Send webhook
curl -X POST \
  https://{api-id}.execute-api.us-east-1.amazonaws.com/webhook/github \
  -H "X-Hub-Signature-256: sha256=$signature" \
  -H "X-GitHub-Event: test" \
  -d "$payload"
```

## Troubleshooting

### Lambda Errors

```bash
# View logs
aws logs tail /aws/lambda/agentmux-webhook-router-prod --follow

# Test directly
aws lambda invoke \
  --function-name agentmux-webhook-router-prod \
  --payload '{"requestContext":{"http":{"method":"GET","path":"/health"}}}' \
  response.json
```

### Connection Issues

```bash
# Check if connection exists
aws dynamodb query \
  --table-name AgentMuxConnections-prod \
  --index-name WorkspaceIndex \
  --key-condition-expression "workspaceId = :wid" \
  --expression-attribute-values '{":wid":{"S":"your-workspace"}}'
```

## Development Roadmap

### Phase 1: Infrastructure (✅ Current)
- [x] CDK stack definition
- [x] Lambda webhook router
- [x] DynamoDB tables
- [x] API Gateway (HTTP + WebSocket)
- [x] Deployment documentation

### Phase 2: AgentMux Integration (Next)
- [ ] Go WebSocket client (`pkg/webhookinjector/`)
- [ ] Terminal registration
- [ ] PTY command injection
- [ ] Configuration file loader

### Phase 3: Frontend UI
- [ ] Webhook configuration modal
- [ ] Subscription management
- [ ] Real-time status indicators
- [ ] Template builder

### Phase 4: Enhancements
- [ ] Bidirectional communication (terminal → webhook)
- [ ] Rich notifications (desktop alerts)
- [ ] Multi-workspace coordination
- [ ] Interactive confirmation prompts
- [ ] Command history/replay

## Related Documentation

- **[cdk/README.md](cdk/README.md)** - CDK-specific documentation
- **[lambda/webhook-router/handler.py](lambda/webhook-router/handler.py)** - Lambda implementation

## Support

- **Issues:** https://github.com/agentmuxhq/agentmux/issues

---

**Status:** ✅ Ready for deployment
**Version:** 1.0.0
**Last Updated:** 2025-10-29
