# @a5af/agentmux-server

WebSocket server for multi-agent communication. Runs on bastion EC2 instance.

## Features

- WebSocket server with JWT authentication
- Message persistence in DynamoDB (7-day TTL)
- Agent registry and presence tracking
- Real-time message delivery with fallback to persistence
- Broadcast messages to all agents
- Health monitoring and graceful shutdown

## Installation

```bash
npm install
npm run build
```

## Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3100` | WebSocket server port |
| `AWS_REGION` | `us-east-1` | AWS region |
| `MESSAGES_TABLE` | `agentmux-messages-prod` | DynamoDB messages table |
| `AGENTS_TABLE` | `agentmux-agents-prod` | DynamoDB agents table |
| `SECRET_NAME` | `services/infra` | Secrets Manager secret name |

## JWT Secret Setup

The server loads the JWT secret from AWS Secrets Manager (`services/infra`). Add the secret:

```bash
# Get current secret
CURRENT=$(aws secretsmanager get-secret-value --secret-id services/infra --query SecretString --output text)

# Add agentmux-jwt-secret
UPDATED=$(echo "$CURRENT" | jq '.["agentmux-jwt-secret"] = "YOUR_RANDOM_SECRET_HERE"')

# Update secret
aws secretsmanager update-secret --secret-id services/infra --secret-string "$UPDATED"
```

## Running

```bash
npm start
```

Or with environment variables:

```bash
PORT=3100 MESSAGES_TABLE=agentmux-messages-prod npm start
```

## WebSocket Protocol

### Authentication

Connect with JWT token:

```
ws://localhost:3100?token=YOUR_JWT_TOKEN
```

Or via header:

```
Authorization: Bearer YOUR_JWT_TOKEN
```

### Message Types

**Send Message:**
```json
{
  "type": "send_message",
  "to": "agent2",
  "message": "Hello",
  "priority": "normal"
}
```

**Read Messages:**
```json
{
  "type": "read_messages",
  "unread_only": true,
  "limit": 100
}
```

**List Agents:**
```json
{
  "type": "list_agents"
}
```

**Broadcast:**
```json
{
  "type": "broadcast_message",
  "message": "Hello all",
  "priority": "normal"
}
```

**Delete Messages:**
```json
{
  "type": "delete_messages",
  "message_ids": ["msg-id-1", "msg-id-2"]
}
```

**Ping:**
```json
{
  "type": "ping"
}
```

## IAM Permissions Required

The EC2 instance role needs:

- `dynamodb:PutItem`, `dynamodb:Query` on messages table
- `dynamodb:UpdateItem`, `dynamodb:Query` on agents table
- `secretsmanager:GetSecretValue` on `services/infra` secret

## systemd Service

See `../../../infrastructure/` for deployment scripts.

## License

MIT
