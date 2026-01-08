# AgentMux Streamable HTTP Implementation Spec

## Overview

Replace the current stdio wrapper pattern with native MCP Streamable HTTP transport in Lambda.

**Current Architecture:**
```
Claude Code <--stdio--> agentmux-client <--HTTP POST--> Lambda
```

**Proposed Architecture:**
```
Claude Code <--Streamable HTTP--> Lambda (direct)
```

## Why Consider This?

| Aspect | Current (stdio wrapper) | Streamable HTTP |
|--------|------------------------|-----------------|
| Complexity | Extra npm package | Native Lambda |
| Latency | +1 hop (wrapper) | Direct |
| Streaming | Not supported | Native SSE |
| MCP Standard | Custom proxy | Official spec |
| Maintenance | Two codebases | One codebase |

## AWS Resources Required

### Minimal Setup (Lambda Function URL)

| Resource | Purpose | Cost |
|----------|---------|------|
| **Lambda** | MCP server | ~$0.20/1M requests (128MB, 100ms avg) |
| **CloudFront** | Custom domain, caching | ~$0.085/10K requests + $0.01/GB |
| **ACM Certificate** | SSL for custom domain | Free |
| **Route 53** | DNS alias | $0.50/hosted zone/month |

**Estimated monthly cost (1M requests):** ~$1-2/month

### Full Setup (with OAuth)

| Resource | Purpose | Cost |
|----------|---------|------|
| All above | - | ~$1-2/month |
| **API Gateway** | OAuth integration | $3.50/1M requests |
| **Cognito User Pool** | OAuth provider | Free tier: 50K MAU |
| **Cognito App Client** | Client credentials | Included |

**Estimated monthly cost (1M requests):** ~$5-10/month

## Implementation Options

### Option 1: Lambda Function URL + SigV4 (Simplest)

Uses IAM authentication instead of bearer tokens.

**Pros:**
- No additional AWS resources
- Built-in request signing
- Works with AWS SDK

**Cons:**
- Requires AWS credentials on client
- Not suitable for external clients

**Lambda Changes:**
```typescript
// lambda.ts - Add Streamable HTTP handler
export const handler = async (event: APIGatewayProxyEventV2) => {
  const method = event.requestContext.http.method;

  if (method === 'GET') {
    // SSE stream for server->client notifications
    return handleSSEStream(event);
  }

  if (method === 'POST') {
    // JSON-RPC request handling
    return handleMCPRequest(event);
  }

  return { statusCode: 405 };
};
```

### Option 2: Lambda Function URL + Bearer Token (Current + Streamable)

Keep bearer token auth, add Streamable HTTP support.

**Pros:**
- Backward compatible
- Simple auth model
- Works with CloudFront

**Cons:**
- No SSE streaming (Lambda doesn't support)
- Must use response URLs for streaming

**Implementation:**
```typescript
// Streamable HTTP without SSE (JSON responses only)
export const handler = async (event) => {
  const sessionId = event.headers['mcp-session-id'];

  // Parse JSON-RPC request
  const request = JSON.parse(event.body);

  // Handle MCP request
  const result = await handleMCPMethod(request.method, request.params);

  return {
    statusCode: 200,
    headers: {
      'Content-Type': 'application/json',
      'Mcp-Session-Id': sessionId || generateSessionId(),
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: request.id,
      result,
    }),
  };
};
```

### Option 3: API Gateway + Lambda (Full Streamable HTTP)

Use API Gateway for proper SSE streaming.

**Pros:**
- Full MCP Streamable HTTP compliance
- Native SSE support via WebSocket API
- OAuth integration

**Cons:**
- More complex setup
- Higher cost
- WebSocket API for SSE is overkill

### Option 4: AWS Labs Library (Recommended)

Use `@aws/run-mcp-servers-with-aws-lambda` library.

**Pros:**
- Battle-tested implementation
- Handles stdio<->HTTP bridging
- Supports multiple auth methods

**Cons:**
- Dependency on AWS library
- Packages MCP server inside Lambda

## Recommended Approach: Option 2 (Enhanced Current)

Given our constraints:
1. Lambda Function URLs don't support true SSE
2. We don't need server->client streaming (our MCP is request/response only)
3. Bearer token auth is sufficient

**Enhance current Lambda to support Streamable HTTP transport directly:**

### Changes Required

#### 1. Lambda Handler Updates

```typescript
// server/src/lambda.ts

import { APIGatewayProxyEventV2, APIGatewayProxyResultV2 } from 'aws-lambda';

// Session storage (use DynamoDB for production)
const sessions = new Map<string, SessionState>();

export const handler = async (event: APIGatewayProxyEventV2): Promise<APIGatewayProxyResultV2> => {
  const method = event.requestContext.http.method;
  const path = event.rawPath;

  // Auth check
  const authResult = await validateAuth(event.headers.authorization);
  if (!authResult.valid) {
    return { statusCode: 401, body: 'Unauthorized' };
  }

  // MCP endpoint
  if (path === '/mcp') {
    if (method === 'POST') {
      return handleMCPPost(event, authResult.agentId);
    }
    if (method === 'GET') {
      // Return 405 - we don't support SSE streaming in Lambda
      return { statusCode: 405, body: 'SSE not supported' };
    }
    if (method === 'DELETE') {
      return handleSessionTerminate(event);
    }
  }

  // Legacy JSON-RPC endpoint (backward compatible)
  if (path === '/mcp' && method === 'POST') {
    return handleLegacyMCP(event, authResult.agentId);
  }

  return { statusCode: 404 };
};

async function handleMCPPost(event: APIGatewayProxyEventV2, agentId: string) {
  const sessionId = event.headers['mcp-session-id'];
  const accept = event.headers['accept'] || '';

  const request = JSON.parse(event.body || '{}');

  // Handle initialization
  if (request.method === 'initialize') {
    const newSessionId = generateSessionId();
    sessions.set(newSessionId, { agentId, created: Date.now() });

    return {
      statusCode: 200,
      headers: {
        'Content-Type': 'application/json',
        'Mcp-Session-Id': newSessionId,
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: request.id,
        result: {
          protocolVersion: '2025-03-26',
          capabilities: { tools: {} },
          serverInfo: { name: 'agentmux', version: '2.0.0' },
        },
      }),
    };
  }

  // Validate session for non-init requests
  if (!sessionId || !sessions.has(sessionId)) {
    return { statusCode: 400, body: 'Invalid or missing session' };
  }

  // Handle tool calls
  const result = await handleToolCall(request, agentId);

  return {
    statusCode: 200,
    headers: {
      'Content-Type': 'application/json',
      'Mcp-Session-Id': sessionId,
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: request.id,
      result,
    }),
  };
}
```

#### 2. CDK Updates

```typescript
// No changes needed - current Lambda Function URL works
// CloudFront already configured for agentmux.asaf.cc
```

#### 3. Client Configuration

```json
// .mcp.json - Direct HTTP (no stdio wrapper)
{
  "mcpServers": {
    "agentmux": {
      "type": "http",
      "url": "https://agentmux.asaf.cc/mcp",
      "headers": {
        "Authorization": "Bearer {{AGENTMUX_TOKEN}}"
      }
    }
  }
}
```

### Migration Path

1. **Phase 1:** Add Streamable HTTP handler to Lambda alongside legacy
2. **Phase 2:** Update one agent to use HTTP transport
3. **Phase 3:** Validate, then migrate all agents
4. **Phase 4:** Deprecate stdio wrapper
5. **Phase 5:** Remove legacy endpoint

## Cost Comparison

| Setup | Monthly Cost (1M req) | Complexity |
|-------|----------------------|------------|
| Current (stdio) | ~$1.50 | Medium (wrapper) |
| Option 2 (Enhanced) | ~$1.50 | Low (same infra) |
| Option 3 (API GW) | ~$5.00 | High |
| Option 4 (AWS Labs) | ~$1.50 | Medium |

## Decision

**Recommendation:** Option 2 - Enhance current Lambda

**Rationale:**
1. No new AWS resources needed
2. Same cost as current
3. Removes stdio wrapper dependency
4. MCP Streamable HTTP compliant (minus SSE, which we don't need)
5. Backward compatible migration path

## Implementation Estimate

| Task | Effort |
|------|--------|
| Update Lambda handler | 2-3 hours |
| Add session management | 1-2 hours |
| Update tests | 1-2 hours |
| Update client configs | 1 hour |
| Documentation | 1 hour |
| **Total** | **6-9 hours** |

## Open Questions

1. Do we need SSE streaming for any use case?
2. Should we support both transports permanently or deprecate stdio?
3. Session storage: in-memory (current Lambda) vs DynamoDB (persistent)?
