# AgentMux Auth Outage Retrospective
**Date:** 2026-01-13
**Duration:** ~2 hours (estimated)
**Impact:** All agent-to-agent communication blocked
**Severity:** High

## Summary
AgentMux authentication stopped working for all agents. The `JWT_SECRET` environment variable was missing from the Lambda function, and the server code was using `agentmux-api-key` (simple string comparison) instead of JWT verification.

## Timeline
- **Unknown time:** JWT_SECRET env var removed from Lambda (likely during a CDK deployment)
- **~20:00 UTC:** AgentC setup attempted, reported auth failures
- **22:15 UTC:** Investigation started
- **22:19 UTC:** Added JWT_SECRET to Lambda - still failed
- **22:20 UTC:** Discovered server uses `agentmux-api-key`, not JWT verification
- **22:21 UTC:** Confirmed API key works, updated AgentC config
- **22:21 UTC:** Service restored

## Root Cause Analysis

### Primary Cause
The Lambda was missing the `JWT_SECRET` environment variable. However, this was a **red herring** - the actual server code never validates JWTs.

### Actual Issue
The server code (`agentmux/server/src/index.ts`) uses simple API key comparison:
```typescript
// Line 23 - Fetches API key from Secrets Manager
cachedApiKey = secrets["agentmux-api-key"];

// Line 40 - Simple string comparison (no JWT verification)
if (authHeader !== `Bearer ${expectedKey}`)
```

The per-agent JWT tokens (`agentmux-jwt-agentx`, `agentmux-jwt-agent1`, etc.) are **never used** by the server. All agents should use the same `agentmux-api-key`.

### How It Broke
1. Original deployment had `JWT_SECRET` in Lambda env vars
2. Sometime later, Lambda was redeployed (likely via CDK) without `JWT_SECRET`
3. The `getApiKey()` function was still working (fetches from Secrets Manager)
4. But the Lambda cold start logs suggested JWT_SECRET was needed
5. This led to confusion during debugging

### Why MCP Clients Were Configured with JWTs
The `.mcp.json` files were configured with per-agent JWT tokens:
```json
"AGENTMUX_TOKEN": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."
```

But these should have been:
```json
"AGENTMUX_TOKEN": "Lc7qWbUWL/0lwYhNuCX0OqxpgIoCY6CaRizUMb16PXQ="  // API key
```

## Resolution
1. Updated `C:\Systems\.mcp.json` for AgentC to use `agentmux-api-key`
2. Verified AgentX already had correct API key
3. Confirmed all communication restored
4. **Removed JWT infrastructure entirely** (decision: Option B)

## Action Items - COMPLETED

### Immediate (All Done)
- [x] Verify all agent `.mcp.json` files use `agentmux-api-key` not JWT tokens
- [x] Update claw MCP deployment template to use API key
- [x] Remove unused JWT tokens from `services/infra` secret (cleanup)

### JWT Infrastructure Removal (All Done)
- [x] Removed `JWT_SECRET` from Lambda environment variables
- [x] Removed 10 JWT entries from `services/infra`:
  - `agentmux-jwt-secret`
  - `agentmux-jwt-agentx`, `agentmux-jwt-agenty`
  - `agentmux-jwt-agent1` through `agentmux-jwt-agent5`
  - `agentmux-jwt-agentc`, `agentmux-jwt-agentg`
- [x] Updated `agentmux/README.md` with clear API key auth documentation
- [x] Secret version bumped to 1.0.13

### Remaining
- [ ] Add `/health` endpoint that doesn't require auth (for monitoring)
- [ ] Add Lambda alarm for auth failure rate spike

## Lessons Learned
1. **Document auth mechanism:** The server's actual auth mechanism (API key) differed from what configs suggested (JWTs)
2. **Test after CDK deployments:** Lambda env vars can be overwritten by CDK
3. **Health endpoints should be unauthenticated:** `/api/health` exists but we were hitting `/health`
4. **Don't assume JWT presence means JWT validation:** Always verify server code

## Affected Components
- `agentmux-server` Lambda function
- All agent MCP configurations
- `services/infra` secret

## Files Changed
- `C:\Systems\.mcp.json` - Updated token to API key
- `agentmux/README.md` - Added Authentication section documenting API key usage
- `services/infra` secret - Removed 10 JWT entries, now v1.0.13
- Lambda `agentmux-server` env vars - Removed JWT_SECRET (was added then removed)
