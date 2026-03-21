# AgentMux Identity Management — Design Spec

**Status:** Draft
**Date:** 2026-03-21
**Author:** AgentY

---

## 1. Problem Statement

Agents running in AgentMux operate under multiple external identities simultaneously. A single agent might:

- Push code as **`agent1-workflow`** (GitHub, limited CI/CD access)
- Read secrets as an **AWS IAM role** (`reagent-lambda-role`)
- Call the Anthropic API with a **per-agent API key**
- Post to Slack under a **bot token**

Today, none of this is visible in AgentMux. Identity is scattered across:
- Env vars in `forge-seed.json` (opaque, no validation)
- `.mcp.json` env blocks (per-workspace, unmanaged)
- AWS `~/.aws/credentials` (system-level, no per-agent scoping)
- Secrets Manager (cloud, no UI)

When something fails — a PR review that can't push, a Lambda that can't read secrets — there's no centralized place to diagnose "which identity did this agent use, and is it still valid?"

### What We're Missing

| Gap | Impact |
|-----|--------|
| No account registry | Can't see which external accounts exist |
| No agent→account mapping UI | Don't know which agent uses which identity |
| No validation layer | Expired tokens cause silent failures |
| No audit surface | Can't tell which agent performed a privileged action |
| No cross-agent identity view | Can't detect conflicts (two agents using same account) |

---

## 2. Scope

**In scope:**
- Defining and storing external account credentials per-agent
- Validating token/key health with live status indicators
- Assigning accounts to agents at forge-seed/config level
- Surfacing identity status in the UI

**Out of scope (v1):**
- OAuth flows (user-delegated GitHub/Slack auth)
- Secret rotation automation
- Fine-grained permission auditing beyond "which agent used which account"
- Cloud team sync (single-user desktop app for now)

---

## 3. Industry Context

The 2025 consensus on agent identity (CSA, OpenID Foundation, Microsoft Entra Agent ID):

1. **Agents are first-class identities** — treat them the same as human users in IAM
2. **Least privilege** — each agent gets the narrowest credentials for its task
3. **JIT credentials** — prefer ephemeral over long-lived tokens where possible
4. **Full audit trail** — log which agent acted under which identity and when
5. **Separation of concerns** — don't build custom auth into each system; centralize it

AWS Bedrock AgentCore Identity (2025) is the closest analog: a unified directory of agent workload identities with per-agent OAuth flows and API key management. We're building a desktop-native version of this for the AgentMux context.

**Key insight from industry:** The hardest problem isn't storage — it's *visibility*. Users need to see at a glance "Agent2 is GitHub `agent2-workflow`, valid, scoped to `repo`. Agent4's AWS role expired 2 hours ago." That's the gap we close.

---

## 4. Identity Model

### 4.1 Account

An `Account` is a named credential for one external provider:

```typescript
interface Account {
  id: string;                    // UUID, generated
  name: string;                  // "GitHub agent1-workflow"
  provider: AccountProvider;     // "github" | "aws" | "anthropic" | "custom"
  kind: AccountKind;             // "pat" | "role" | "api_key" | "env"

  // Display metadata
  display_name?: string;         // "agent1-workflow" (GitHub username, ARN alias, etc.)
  icon?: string;                 // Provider icon, auto-derived from provider
  color?: string;                // For visual badge in pane (auto or user-set)

  // Credential storage reference (never stored plaintext)
  secret_ref: SecretRef;         // Where the credential lives

  // Provider-specific context (non-sensitive)
  context: AccountContext;

  // Health
  status: AccountStatus;
  last_validated_at?: string;    // ISO timestamp
  expires_at?: string;           // For tokens with known expiry
  validation_error?: string;

  // Assignment
  assigned_agents: string[];     // Forge agent IDs

  created_at: string;
  updated_at: string;
}

type AccountProvider = "github" | "aws" | "anthropic" | "slack" | "custom";
type AccountKind = "pat" | "role" | "api_key" | "env_ref";
type AccountStatus = "valid" | "expired" | "invalid" | "unknown" | "checking";

interface SecretRef {
  backend: "secrets_manager" | "env" | "keychain" | "plaintext_dev";
  // secrets_manager: path like "<secret-store>" + json path
  sm_path?: string;
  sm_json_path?: string;
  // env: read from env var
  env_var?: string;
  // keychain: OS keychain key
  keychain_key?: string;
}

interface AccountContext {
  // GitHub
  github_username?: string;
  github_scopes?: string[];      // ["repo", "workflow", "read:org"]

  // AWS
  aws_profile?: string;
  aws_role_arn?: string;
  aws_region?: string;
  aws_account_id?: string;

  // Anthropic
  anthropic_model?: string;
  anthropic_org?: string;

  // Custom
  endpoint?: string;
  description?: string;
}
```

### 4.2 AgentIdentity

The runtime link between an agent and its accounts:

```typescript
interface AgentIdentity {
  agent_id: string;              // Forge agent ID (e.g. "AgentY", "agent1")
  accounts: AccountRef[];        // Ordered list of assigned accounts
  env_injections: EnvInjection[]; // Env vars injected at agent launch
}

interface AccountRef {
  account_id: string;
  role: "primary" | "fallback";  // primary = default; fallback = if primary fails
}

interface EnvInjection {
  // How the account's credential maps to an env var in the agent's shell
  account_id: string;
  env_var: string;               // e.g. "GH_TOKEN", "AWS_PROFILE", "ANTHROPIC_API_KEY"
  transform?: "raw" | "aws_profile_name"; // Optional value transform
}
```

### 4.3 Validation

For each provider, the backend runs a health check:

| Provider | Validation Method | Frequency |
|----------|------------------|-----------|
| `github` | `GET /user` with token, check status 200 + scopes header | On load, on demand, every 30 min |
| `aws` | `sts:GetCallerIdentity` | On load, on demand |
| `anthropic` | `GET /v1/models` with API key | On load, on demand |
| `custom` | HTTP GET to `context.endpoint` (if set) | On demand only |

Validation is non-blocking. Status stays `"unknown"` until the first check completes.

---

## 5. Where It Lives in the UI

### 5.1 Decision: New Pane vs. Part of Forge vs. Part of Swarm

**Option A — New `identity` pane**
- Standalone pane like `swarm`, `forge`, `term`
- Users open it when they need to manage accounts
- Pros: full surface, cross-agent view, resizable
- Cons: another pane type to remember; identity is config, not live monitoring

**Option B — Section inside `forge` pane**
- "Identity" tab/section when editing a Forge agent
- Pros: right context (per-agent config), no new pane type
- Cons: no cross-agent view; can't see account health independent of agent

**Option C — Hybrid (recommended)**
- **Global identity pane** (`identity` pane type) — account registry, cross-agent assignment matrix, health dashboard
- **Forge identity section** — lightweight view within an agent's Forge card: "Assigned accounts → [badge] GitHub agent1-workflow ✓ [badge] AWS dev-role ✓"
- **Swarm identity badges** — small identity indicators on agent cards in the Swarm overview

**Rationale for hybrid:**
- Identity has two modes: *configuration* (set it and forget it → Forge) and *observability* (is it working right now → identity pane + swarm)
- The global pane handles cross-cutting concerns (see all accounts, spot conflicts, rotate)
- Forge integration keeps the configuration workflow in context
- Swarm integration closes the observability loop without opening a new pane

### 5.2 Identity Pane Layout

```
┌─────────────────────────────────────────────────────────────────┐
│ IDENTITY                                          [+ Add Account]│
│ ─────────────────────────────────────────────────────────────── │
│ [Accounts ▼]  [Assignments]  [Audit Log]                        │
│                                                                  │
│  GITHUB ─────────────────────────────────────────────────       │
│  [GH] agent1-workflow      ✓ valid  repo workflow    Agent1, 2   │
│  [GH] AgentX-asaf          ✓ valid  repo read:org    AgentX      │
│  [GH] a5af (admin)         ✓ valid  admin            (unassigned)│
│                                                                  │
│  AWS ────────────────────────────────────────────────           │
│  [AWS] dev-role             ✓ valid  sts:AssumeRole  Agent1-5    │
│  [AWS] reagent-lambda-role  ✗ expired  —             (unassigned)│
│                                                                  │
│  ANTHROPIC ──────────────────────────────────────────           │
│  [AI] claude-agents-key     ✓ valid  claude-sonnet-4 All         │
│                                                                  │
│  ─────────────────────────────────────────────────────────────  │
│  ▶ Assignments                                                   │
│                                                                  │
│  Agent       GitHub              AWS              Anthropic      │
│  AgentX      AgentX-asaf ✓      dev-role ✓       agents-key ✓  │
│  AgentY      AgentX-asaf ✓      dev-role ✓       agents-key ✓  │
│  Agent1      agent1-workflow ✓   dev-role ✓       agents-key ✓  │
│  Agent2      agent1-workflow ✓   dev-role ✓       agents-key ✓  │
└─────────────────────────────────────────────────────────────────┘
```

**Accounts tab:**
- Grouped by provider (GitHub, AWS, Anthropic, Custom)
- Each row: [provider icon] [name] [status badge] [scopes] [assigned agents]
- Click to open detail panel (right side or expand row)
- Status badge: colored dot + "valid" / "expired" / "invalid" / "checking…"

**Detail panel (when account selected):**
```
┌──────────────────────────────────────────┐
│ GitHub: agent1-workflow                  │
│ ● valid · last checked 4 min ago        │
│                                          │
│ Username:   agent1-workflow              │
│ Scopes:     repo  workflow  read:org     │
│ Expires:    Never (PAT)                  │
│ Secret:     <secret-store> → .gh-token   │
│                                          │
│ Assigned to:                             │
│   Agent1  Agent2                         │
│                                          │
│ Env injection: GH_TOKEN                  │
│                                          │
│ [Validate Now]  [Edit]  [Remove]         │
└──────────────────────────────────────────┘
```

**Assignments tab:**
- Matrix: rows = agents, columns = providers
- Cell: account badge + status dot
- Click cell to change assignment

**Audit Log tab:**
- Timestamped events: "Agent1 launched with GitHub agent1-workflow at 14:32"
- Filter by agent, provider, date

### 5.3 Forge Integration

When viewing/editing an agent in the Forge pane, add an "Identity" section below env vars:

```
┌────────────────────────────────────────────────┐
│ IDENTITY                                  [Edit]│
│ ─────────────────────────────────────────────  │
│ [GH] agent1-workflow  ✓  →  GH_TOKEN           │
│ [AWS] dev-role        ✓  →  AWS_PROFILE=dev     │
│ [AI]  agents-key      ✓  →  ANTHROPIC_API_KEY   │
└────────────────────────────────────────────────┘
```

Clicking "Edit" opens the account assignment dialog (not the full identity pane).

### 5.4 Swarm Integration

In the Swarm overview grid, each agent card shows identity badges:

```
┌─────────────────────────┐
│ Agent1  ● active        │
│ working on feature/auth │
│ ─────────────────────── │
│ [GH]✓ [AWS]✓ [AI]✓      │
│ 42 tool calls · 14k tok  │
└─────────────────────────┘
```

Hovering a badge shows: provider name, account name, status.

---

## 6. Data Storage

### 6.1 Account Definitions

Stored in `agentmuxsrv-rs` SQLite (via WaveStore) — same pattern as other persistent objects.

New WaveObj type:
```rust
pub const ACCOUNT_OBJ_TYPE: &str = "account";
pub const AGENT_IDENTITY_OBJ_TYPE: &str = "agent-identity";
```

New RPC endpoints:
```
account.List        → Vec<Account>
account.Get         → Account
account.Create      → Account
account.Update      → Account
account.Delete      → ()
account.Validate    → AccountStatus  (triggers background validation)

agent-identity.Get  → AgentIdentity
agent-identity.Set  → AgentIdentity
agent-identity.List → Vec<AgentIdentity>
```

### 6.2 Credentials

Credentials are **never stored in AgentMux's SQLite**. The `SecretRef` points to:

| Backend | Description | Use Case |
|---------|-------------|----------|
| `secrets_manager` | AWS Secrets Manager (via `secrets` CLI) | Team/cloud setups |
| `env` | Read from env var at validation/launch time | Simple local setups |
| `keychain` | OS keychain (macOS Keychain, Windows DPAPI, libsecret Linux) | Single-user desktop |
| `plaintext_dev` | Stored in WaveStore (dev only, clearly labeled dangerous) | Quick prototyping |

For AgentMux's use case (Claw/private deployment), `secrets_manager` is the primary backend.

### 6.3 Forge-seed.json Extension

For the Claw deployment, identity can be declared in `forge-seed.json` alongside the agent definition:

```json
{
  "version": 2,
  "agents": [
    {
      "id": "AgentY",
      "name": "Agent Y",
      "content": {
        "env": ["AGENTMUX_AGENT_ID=AgentY"]
      },
      "identity": {
        "accounts": ["agentx-github", "dev-aws-role", "claude-agents-key"],
        "env_injections": [
          { "account": "agentx-github", "env_var": "GH_TOKEN" },
          { "account": "dev-aws-role", "env_var": "AWS_PROFILE", "value": "dev" },
          { "account": "claude-agents-key", "env_var": "ANTHROPIC_API_KEY" }
        ]
      }
    }
  ],
  "accounts": [
    {
      "id": "agentx-github",
      "name": "GitHub AgentX-asaf",
      "provider": "github",
      "kind": "pat",
      "display_name": "AgentX-asaf",
      "secret_ref": {
        "backend": "secrets_manager",
        "sm_path": "<secret-store>",
        "sm_json_path": "github-token-agentx"
      },
      "context": {
        "github_username": "AgentX-asaf",
        "github_scopes": ["repo", "workflow", "read:org"]
      }
    },
    {
      "id": "dev-aws-role",
      "name": "AWS Dev Role",
      "provider": "aws",
      "kind": "role",
      "secret_ref": {
        "backend": "env",
        "env_var": "AWS_PROFILE"
      },
      "context": {
        "aws_profile": "dev",
        "aws_region": "us-east-1"
      }
    }
  ]
}
```

---

## 7. Implementation Architecture

### 7.1 Backend (agentmuxsrv-rs)

**New module:** `src/backend/identity/`

```
identity/
├── mod.rs              # Module re-exports
├── store.rs            # CRUD on Account and AgentIdentity via WaveStore
├── validator.rs        # Async validation tasks per provider
├── github.rs           # GitHub token validation (GET /user)
├── aws.rs              # AWS STS GetCallerIdentity
├── anthropic.rs        # Anthropic /v1/models health check
├── injector.rs         # Build EnvInjection → actual env vars at agent launch
└── forge_seed.rs       # Parse accounts[] from forge-seed.json
```

**Validator** runs in background (tokio task):
- On startup: validate all accounts with `status != "checking"`
- On demand: `account.Validate` RPC triggers immediate re-check
- Periodic: check accounts with `expires_at` within 1 hour, every 5 min for expired/invalid

**Injector** hooks into agent launch (in `blockcontroller/shell.rs`):
```rust
// Before spawning agent subprocess:
let identity = identity_store.get_agent_identity(agent_id).await?;
for injection in &identity.env_injections {
    let value = secret_resolver.resolve(&injection.secret_ref).await?;
    cmd.env(&injection.env_var, value);
}
```

### 7.2 Frontend

New pane type: `identity`

```
frontend/app/view/identity/
├── identity.tsx           # Barrel
├── identity-model.ts      # IdentityViewModel (Solid.js signals)
├── identity-view.tsx      # Three-tab layout
├── account-list.tsx       # Accounts grouped by provider
├── account-detail.tsx     # Detail panel (right side)
├── assignment-matrix.tsx  # Agent × provider matrix
├── audit-log.tsx          # Audit log tab
└── identity-view.scss
```

**IdentityViewModel:**
```typescript
class IdentityViewModel implements ViewModel {
  viewType = "identity";
  viewIcon = "id-card";
  viewName = () => "Identity";

  tab = createSignal<"accounts" | "assignments" | "audit">("accounts");
  accounts = createSignal<Account[]>([]);
  agentIdentities = createSignal<AgentIdentity[]>([]);
  selectedAccount = createSignal<Account | null>(null);

  async loadAccounts() { ... }
  async validateAccount(id: string) { ... }
  async updateAssignment(agentId: string, accountId: string) { ... }
}
```

**Widget registration** (in `block.tsx`):
```typescript
registry.register("identity", {
  viewModel: IdentityViewModel,
  icon: "id-card",
  label: "Identity",
  description: "External account and credential management",
  color: "#a78bfa", // violet
});
```

**Event subscriptions:**
- `identity:account-validated` — update status badge without reload
- `identity:account-expired` — flash expired badge in Swarm/Forge views

### 7.3 Forge Pane Integration

Add to `forge-view.tsx` — below the "Environment" section, before "Skills":

```tsx
<ForgeSection title="Identity" icon="id-card">
  {agentIdentities.map(identity => (
    <IdentityBadge
      account={getAccount(identity.account_id)}
      envVar={identity.env_var}
    />
  ))}
  <Button variant="ghost" onClick={openIdentityPane}>
    Manage →
  </Button>
</ForgeSection>
```

### 7.4 Swarm Integration

In `swarm-view.tsx` agent cards, add identity row:

```tsx
<div class="agent-card-identity">
  {agentAccounts.map(a => (
    <ProviderBadge
      provider={a.provider}
      status={a.status}
      title={`${a.name}: ${statusLabel(a.status)}`}
    />
  ))}
</div>
```

---

## 8. UX Details

### Status Badges

| Status | Color | Icon | Label |
|--------|-------|------|-------|
| `valid` | green | ● | "valid" |
| `checking` | amber (pulsing) | ◌ | "checking…" |
| `expired` | red | ✗ | "expired" |
| `invalid` | red | ✗ | "invalid" |
| `unknown` | gray | ○ | "unknown" |

### Error States

- **Expired token:** Red badge + tooltip "Expired 2h ago. Click to re-validate or rotate."
- **Network error during validation:** Amber badge + "Could not reach GitHub API"
- **Secret not found:** Red badge + "Secret path not found in Secrets Manager"
- **Agent launch blocked:** If required account is `invalid` or `expired`, show warning before launching (non-blocking — user can proceed)

### Progressive Disclosure

- Account list shows: name, status, provider, assigned agents (summary)
- Click to see: scopes, secret reference, env injection, full validation history
- Edit mode: inline form within detail panel
- Add account: drawer/modal (not a full pane navigation)

### Security UX

- **Never show credential values** — show only `•••••••••••` with a "Copy" button (copies resolved value for 30s)
- **Secret reference is visible** (e.g. `<secret-store> → .gh-token`) — lets users verify configuration without exposing the value
- **"Plaintext (dev)" accounts** shown with ⚠ warning and red border — discourage in production
- **Audit log** always accessible, always appended-to (no delete)

---

## 9. Phased Rollout

### Phase 1 — Account Registry (no validation, no injection)
- Account CRUD (SQLite storage)
- Identity pane with Accounts tab (display only)
- Forge-seed.json parsing for initial accounts
- Status always `"unknown"` until Phase 2

### Phase 2 — Validation
- Background validator (GitHub, AWS, Anthropic)
- Status badges update live
- `identity:account-validated` events → Swarm badge updates

### Phase 3 — Agent Assignment + Injection
- AgentIdentity assignments in Forge pane
- Env injection at agent launch time
- Assignment matrix in Identity pane

### Phase 4 — Audit + Rotation
- Audit log (track which agent launched with which accounts)
- Expiry warnings (toast + badge pulse before expiry)
- Manual rotation trigger ("Rotate token" → calls secrets CLI)

---

## 10. Open Questions

1. **Should accounts be global (workspace-level) or per-agent-config?** Recommendation: global registry with per-agent assignment — keeps accounts DRY, avoids drift when rotating.

2. **Keychain backend priority for desktop users?** macOS Keychain / Windows DPAPI are ideal but require OS-level integration. Start with `secrets_manager` + `env` for the Claw deployment; add keychain as Phase 2 enhancement for open-source users.

3. **Should identity pane be a standalone pane or a drawer/sheet?** Full pane gives resize/split capability and feels consistent with other AgentMux views. But a "sheet" (like Settings in VSCode) might feel more appropriate for config. Lean toward standalone pane — aligns with AgentMux's everything-is-a-pane philosophy.

4. **How does the GitHub "layer" system in CLAUDE.md map here?** The three GitHub layers (MCP agent, gh CLI, admin) map directly to three Account entries with different scopes:
   - `agent1-workflow` → Layer 1 (MCP tools)
   - `AgentX-asaf` → Layer 2 (gh CLI)
   - `a5af` → Layer 3 (admin, assigned to no agent by default, only used explicitly)

5. **Container agents (Agent1-5, Docker):** How are credentials injected into containers? Current approach: env vars at container start via claw. Identity system should generate the env var block for claw to inject — not bypass the container boundary.

---

## References

- [CSA Agentic AI IAM Framework](https://cloudsecurityalliance.org/artifacts/agentic-ai-identity-and-access-management-a-new-approach)
- [OpenID Foundation: Identity Management for Agentic AI](https://arxiv.org/abs/2510.25819)
- [Microsoft Entra Agent ID / Zero-Trust Agents](https://techcommunity.microsoft.com/blog/azure-ai-foundry-blog/zero-trust-agents-adding-identity-and-access-to-multi-agent-workflows/4427790)
- [Amazon Bedrock AgentCore Identity](https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/key-features-and-benefits.html)
- [HashiCorp Vault: AI Agent Identity](https://developer.hashicorp.com/validated-patterns/vault/ai-agent-identity-with-hashicorp-vault)
- AgentMux codebase: `frontend/app/view/forge/`, `frontend/app/view/swarm/`, `agentmuxsrv-rs/src/backend/`
- AgentMux swarm spec: `docs/specs/swarm-analysis.md`
