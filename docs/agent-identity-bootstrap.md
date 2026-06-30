# Agent Identity Bootstrap

Documents how an agent seeds its own identity in AgentMux using credentials
from the `a5af/dev-tools` secrets CLI.

## Steps

1. **Retrieve credential** — pull the agent's PAT from AWS Secrets Manager:
   ```
   node packages/secrets/bin/secrets.js get services/infra --path gh-token-<agent>
   ```

2. **Store in OS keychain** — write to Windows Credential Manager so the
   secret never sits in plaintext in the DB:
   ```
   cmdkey /generic:agentmux /user:acct-<agent>-github-<ts> /pass:<pat>
   ```

3. **Create identity account** — call `upsertidentityaccount` over the local
   WS RPC (`ws://127.0.0.1:$PORT/ws`, `X-AuthKey` header):
   ```json
   {
     "command": "upsertidentityaccount",
     "data": {
       "id": "acct-<agent>-github-<ts>",
       "name": "<Agent> GitHub PAT",
       "provider": "github",
       "kind": "pat",
       "secret_ref": { "backend": "keychain", "service": "agentmux", "account": "acct-<agent>-github-<ts>" }
     }
   }
   ```

4. **Link to agent definition** — call `linkagentidentity`:
   ```json
   {
     "command": "linkagentidentity",
     "data": { "agent_id": "<def-uuid>", "account_id": "acct-<agent>-github-<ts>", "provider": "github" }
   }
   ```

## Notes

- The agent's definition UUID is available in the WS event stream on connect
  (`agentId` field in the block meta).
- `AGENTMUX_AUTH_KEY` and `AGENTMUX_LOCAL_URL` are injected into every agent's
  environment at spawn time, so the WS call works without additional setup.
- `GH_TOKEN` / `GITHUB_TOKEN` env vars may be stale at spawn; prefix `gh`
  commands with `env -u GH_TOKEN -u GITHUB_TOKEN` until the spawn config is
  fixed.
- Once PR #1840 (`feat(app-api): MCP bindings`) ships, the write path will
  move to an `IdentityUpsert` MCP tool — no raw WS call needed.

## First agent bootstrapped

- **AgentY** (`agenty`, def `dedc33bf-b69c-4236-9b34-20bda3ef2738`)
  — GitHub account `AgentY-asaf`, keychain key `acct-agenty-github-1782788341`
  — bootstrapped 2026-06-30
