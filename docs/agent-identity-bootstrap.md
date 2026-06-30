# Agent Identity Bootstrap

Documents how an agent seeds its own identity in AgentMux using credentials
from the `a5af/dev-tools` secrets CLI.

## Steps

1. **Retrieve credential** — pull the agent's PAT from AWS Secrets Manager:
   ```
   secrets get services/infra --path gh-token-<agent>
   ```

2. **Store in OS keychain** — write to Windows Credential Manager so the
   secret is not logged to disk. Note: the PAT is briefly visible in the
   process argument list during `cmdkey` invocation (e.g. in Task Manager or
   `wmic process` output) — it is not persisted to disk but can be observed by
   other processes on the machine at that moment:
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

- **Example** (`<agent-name>`, def `<your-agent-def-uuid>`)
  — GitHub account `<GitHubAccount>`, keychain key `acct-<agent>-github-<ts>`
  — bootstrapped `<date>`
