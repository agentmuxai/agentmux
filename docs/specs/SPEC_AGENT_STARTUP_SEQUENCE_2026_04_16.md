# SPEC: Agent Startup Sequence

**Date:** 2026-04-16
**Status:** implemented — shipped in #408 ("structured agent startup sequence from Forge + Identity"). Verified 2026-08-23: `frontend/app/view/agent/startup/buildStartupPayload.ts` exists and implements this. Note: "Forge" (this spec's own terminology for the agent-config system) was later renamed/refactored elsewhere in the codebase (see ABF/bundle terminology in `CLAUDE.md`) — the underlying startup-injection mechanism this spec designed is live, but some of this doc's Forge-specific naming is no longer current. Not re-verified against a possibly-newer "Brief" terminology doc (commit `ea5c711da`, "pin down Brief = what-loads-at-startup") — flagging that as a related doc worth checking, not confirming a supersession relationship without reading it.

---

## Problem

When an agent session starts, the CLI subprocess (Claude, Codex, Gemini) launches
with zero context about who it is, what accounts it owns, or what its role is. The
user's first message has to carry all of this — or the agent fumbles until it
reads CLAUDE.md (which itself doesn't contain runtime identity or credential info).

Today the launch flow writes static config files (CLAUDE.md, .mcp.json, hooks.json)
to the working directory and sets environment variables. But the **first turn of the
conversation** receives no structured context. The agent doesn't know:

- Its own identity (name, slug, description, role)
- Which accounts/credentials are assigned to it and how to use them
- What project it's operating in or what working directory it has
- What other agents exist in the swarm and how to reach them
- What date it is or what version of AgentMux is running

This information exists across Forge (agent config + content) and Identity
(account assignments) but is never assembled into a startup payload.

---

## Goal

On the first turn of a new session (not `--resume`), automatically inject a
structured **startup message** as the opening user turn before any human input.
The message is assembled from Forge + Identity data and is transparently visible
in the conversation — the user sees exactly what the agent was told.

---

## Design

### 1. Startup Payload Assembly

The startup message is a Markdown document assembled at launch time from the
following sources, in order:

```
# Session Context

## Identity
- **Name:** {{agent.name}}
- **Slug:** {{agent.slug}}
- **Provider:** {{provider.displayName}}
- **Working Directory:** {{workDir}}
- **AgentMux Version:** {{version}}
- **Date:** {{YYYY-MM-DD}}

## Description
{{agent.description}}

## Assigned Accounts
{{#each assignedAccounts}}
### {{provider}} — {{account.name}}
- **Kind:** {{account.kind}} ({{account.provider}})
- **Access:** {{account.secret_ref.backend}} → {{envVarOrPath}}
{{#if context}}
- **GitHub User:** {{context.github_username}}
- **AWS Region:** {{context.aws_region}}
- ...provider-specific context fields
{{/if}}
{{/each}}

{{#if noAccounts}}
No accounts assigned. Use the Identity panel to assign credentials.
{{/if}}

## Forge Startup Instructions
{{forgeContent["startup"]}}

## Peer Agents
{{#each peerAgents}}
- **{{name}}** ({{provider}}) — {{description}}
{{/each}}
```

### 2. Data Sources

| Section | Source | API |
|---------|--------|-----|
| Identity fields | `ForgeAgent` row | Already in `agent-model.ts` at launch time |
| Description | `ForgeAgent.description` | Same |
| Assigned accounts | `ForgeAgent.accounts` JSON + Identity `Account[]` from localStorage | `parseAgentAccounts()` in `identity-model.ts` |
| Account details | `Account` objects keyed by ID | `IdentityViewModel.accounts` signal |
| Startup instructions | `ForgeContent` with `content_type: "startup"` | `RpcApi.GetForgeContentCommand` |
| Peer agents | All `ForgeAgent` rows except self | `RpcApi.GetForgeAgentsCommand` |
| Version | `getApi().getAboutModalDetails().version` | Host API |
| Date | `new Date().toISOString().slice(0, 10)` | Runtime |

### 3. Where It Runs

**Location:** `agent-view.tsx`, inside the `onReadyFn` callback (added in v0.33.200).

**Current state (v0.33.200):** `onReadyFn` fetches `ForgeContent("startup")` and
sends it raw via `handleSendMessage`. This spec replaces that with structured
assembly.

**New flow:**

```
onReady()
  → skip if block.meta["agent:sessionid"] exists (resumed session)
  → assembleStartupPayload(agentId, block, provider)
      → read ForgeAgent (already have it as `currentAgent()`)
      → read assigned accounts from ForgeAgent.accounts + Identity localStorage
      → read ForgeContent("startup") for custom instructions
      → read peer agents from forge agent list (already have `forgeAgents()`)
      → read version from host API
      → template the Markdown document
  → handleSendMessage(payload)
```

### 4. New Module: `buildStartupPayload.ts`

Create `frontend/app/view/agent/startup/buildStartupPayload.ts`:

```typescript
interface StartupPayloadOpts {
    agent: ForgeAgent;
    provider: ProviderDefinition;
    workDir: string;
    version: string;
    accounts: ResolvedAccount[];   // Hydrated from Identity + ForgeAgent.accounts
    peerAgents: ForgeAgent[];      // All agents except self
    startupContent: string | null; // ForgeContent "startup" blob
}

function buildStartupPayload(opts: StartupPayloadOpts): string
```

This is a pure function — no RPC calls, no signals. The caller assembles the
inputs from existing reactive state and passes them in. Testable in isolation.

### 5. Account Resolution

The `ForgeAgent.accounts` field is a JSON string like:
```json
{"github": "acct-1713000000-abc", "aws": null}
```

To hydrate this into useful context for the startup payload:

1. Parse via `parseAgentAccounts()` (already in `identity-model.ts`)
2. For each non-null account ID, look up the `Account` object from the
   Identity store (localStorage)
3. Build a `ResolvedAccount` with the provider, name, kind, and relevant
   context fields (username, region, etc.)
4. **Never include secrets or tokens** — only metadata. The agent accesses
   credentials through the env vars / secret refs at runtime, not through
   the startup message.

```typescript
interface ResolvedAccount {
    provider: string;        // "github", "aws", "anthropic", "custom"
    name: string;            // Human label
    kind: string;            // "pat", "role", "api_key", "env_ref"
    accessMethod: string;    // "env:GITHUB_TOKEN" or "secrets_manager:path"
    context: Record<string, string>;  // Provider-specific metadata
}
```

### 6. Template Variables

Reuse the existing `expandTemplate()` function from `agent-model.ts` for the
custom `startup` content section. Template variables available:

| Variable | Value |
|----------|-------|
| `{{AGENT}}` | agent.name |
| `{{AGENT_SLUG}}` | agent.slug |
| `{{AGENT_ID}}` | agent.id |
| `{{AGENT_DISPLAY}}` | agent.name |
| `{{WORKING_DIR}}` | resolved working directory |
| `{{DATE}}` | YYYY-MM-DD |
| `{{VERSION}}` | AgentMux version string |
| `{{PROVIDER}}` | provider.displayName |

### 7. Conversation Visibility

The startup message is sent via `handleSendMessage()` which:
1. Appends a `user_message` node to the document (visible in conversation)
2. Calls `AgentInputCommand` to send to the subprocess

The user sees the full startup payload in the conversation as the first
message. This is intentional — transparency over magic.

**Visual distinction (future):** A `user_message` node with
`metadata.isStartup: true` could receive distinct styling (muted header,
collapsed by default). Not required for v1.

### 8. Opt-out

If no `ForgeContent("startup")` exists AND the agent has no assigned accounts
AND no description, the startup payload is still sent with the minimal Identity
+ Peer Agents sections. To fully disable startup for an agent, add a
`ForgeContent("startup")` with content `__SKIP__` — the assembly function
checks for this sentinel and returns null.

### 9. Resume Sessions

On `--resume` (session ID exists in block meta), the startup message is NOT
re-sent. The agent already has context from the prior session. This is the
existing guard in `onReadyFn`.

---

## Implementation Plan

### PR 1: `buildStartupPayload` + account resolution

1. Create `frontend/app/view/agent/startup/buildStartupPayload.ts`
   - Pure function, Markdown templating
   - Account resolution helper
2. Create `frontend/app/view/agent/startup/buildStartupPayload.test.ts`
   - Unit tests with mock ForgeAgent + Account data
3. Export `resolveAccounts()` from the startup module

### PR 2: Wire into `onReadyFn`

1. Update `agent-view.tsx` `onReadyFn`:
   - Gather inputs from existing signals (`currentAgent()`, `forgeAgents()`, etc.)
   - Call `buildStartupPayload()`
   - Send via `handleSendMessage()`
2. Update `useAgentControllerStatus.ts` if any timing changes needed

### PR 3: UI polish (optional)

1. Add `metadata.isStartup` to the user_message node
2. Style startup messages with muted header / collapse-by-default
3. Add "Re-send startup" button to the control bar

---

## Non-Goals

- **Injecting secrets into the startup message.** Credentials are accessed via
  env vars and secret refs at runtime, never pasted into conversation.
- **Modifying CLAUDE.md generation.** The startup message is a separate channel
  from the static config files written to disk. CLAUDE.md continues to carry
  soul + agentmd + memory + skills.
- **Backend changes.** All assembly happens in the frontend. The backend
  already has all the APIs needed (GetForgeContent, GetForgeAgents, etc.).
- **Provider-specific startup formats.** v1 sends plain Markdown to all
  providers. Provider-specific structured formats (e.g., system prompts via
  API) are a future enhancement.

---

## Security Considerations

- Account `secret_ref.value` (plaintext_dev secrets) must NEVER appear in the
  startup payload. Only the access method (`env:VAR_NAME`, `sm:path`) is shown.
- The startup message is stored in the filestore (same as all conversation
  history). If the filestore is on shared storage, account metadata (usernames,
  ARNs) is visible. This is acceptable — it's the same data shown in the
  Identity panel UI.

---

## Open Questions

1. **Should the startup message be a system prompt instead of a user turn?**
   Claude Code's `-p` mode doesn't support system prompts via stdin — everything
   is a user turn. For providers that support system prompts natively (future),
   this could be a config option on the provider definition.

2. **Max payload size?** With many accounts and many peer agents, the startup
   message could grow large. Consider truncating peer agents to the 10 most
   recently used, or omitting agents with no description.

3. **Should memory content be included?** Currently memory goes into CLAUDE.md.
   Including it in the startup message too would be redundant. Keep it in
   CLAUDE.md only unless there's a reason to duplicate.
