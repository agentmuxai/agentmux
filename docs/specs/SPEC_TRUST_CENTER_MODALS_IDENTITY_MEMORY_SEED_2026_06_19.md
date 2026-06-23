# SPEC: Trust Center Centered Modals + Identity/Memory Seed

**Date:** 2026-06-19  
**Author:** AgentA  
**Status:** Draft

---

## 1. Overview

Three related initiatives shipped together:

1. **Trust Center modal refactor** — detail views inside the Trust Center (BundleManagerModal) currently open as raw `position:fixed` overlays. Replace with proper centered `<Modal>` panels for focus trapping, backdrop, escape-key dismiss, and consistent chrome.

2. **Identity seed** — one-time operation: create GitHub and AWS identity accounts for agent1–5, agentx, agenty and assign them. Credentials live in AWS Secrets Manager (`services/infra`). Idempotent — skips agents whose account slot is already populated.

3. **Memory seed** — one-time operation: write claw template `.md` files into each agent's native memory folder (`~/.claude/projects/<sanitized-cwd>/memory/`). Source is the `a5af/claw` GitHub repo (`templates/` tree). Idempotent — skip files that already exist.

---

## 2. Trust Center Modal Refactor

### 2.1 Current state

`BundleManagerModal` (`frontend/app/modals/bundle-manager-modal.tsx`) is itself a `<Modal scope="window" size="xl">` (960 px). Inside it, three tab panels are always mounted and toggled via `is-hidden { display: none }`:
- **AccountsManager** — left-nav + account list
- **IdentityManager** — identity/bundle list
- **MemoryManager** — memory bundle list

When the user clicks an item, detail views open via:
- **`accounts-chooser-overlay`** — `position: fixed; inset: 0` div rendered inside AccountsManager. No backdrop, no focus trap, no escape handler.
- **AccountForm** — conditionally rendered inline inside AccountsManager, pushing content down.
- **AgentMuxConnectPanel** — same fixed-overlay pattern as accounts-chooser.

### 2.2 Target state

Each detail view opens in a proper `<Modal>` centered on screen:

| Trigger | Current | Target |
|---|---|---|
| Click account row | `accounts-chooser-overlay` fixed div | `openModal({ kind: "trust-center-account-detail", accountId })` |
| "+ New account" | AccountForm inline expansion | `openModal({ kind: "trust-center-account-form", provider?, prefill? })` |
| Click identity row | (identify exact pattern) | `openModal({ kind: "trust-center-identity-detail", bundleId })` |
| Click memory row | (identify exact pattern) | `openModal({ kind: "trust-center-memory-detail", bundleId })` |
| AgentMux connect | fixed overlay | `openModal({ kind: "trust-center-agentmux-connect" })` |

All new kinds use `size="md"` (520 px) or `size="lg"` (720 px) depending on content. All are `scope="window"` so they stack above BundleManagerModal.

### 2.3 Implementation — modal-layer.ts

Add to `ModalLayerRequest` union in `frontend/app/element/modal-layer.ts`:

```typescript
| { kind: "trust-center-account-detail"; accountId: string }
| { kind: "trust-center-account-form"; provider?: string; prefill?: Partial<IdentityAccount> }
| { kind: "trust-center-identity-detail"; bundleId: string }
| { kind: "trust-center-memory-detail"; bundleId: string }
| { kind: "trust-center-agentmux-connect" }
```

### 2.4 Implementation — modal-dispatch.tsx

Add `requestLabel` and `renderRequest` cases. Each case renders the matching panel component (extract from the existing inline JSX in `AccountsManager`, `IdentityManager`, `MemoryManager`). Panel receives `api.close()` as `onClose` prop.

### 2.5 Implementation — AccountsManager refactor

1. Remove the `accounts-chooser-overlay` `<div>` (and its `position: fixed` SCSS).
2. Remove the inline `AccountForm` conditional render.
3. Replace both with `openModal()` calls (import from `frontend/app/store/modalmodel.ts`).
4. Extract `AccountDetailPanel` and `AccountFormPanel` as standalone panel components to `frontend/app/modals/trust-center/`.
5. Repeat the same extraction for `AgentMuxConnectPanel`.

### 2.6 SCSS

New panel components use the standard modal fragment classes:
- `.modal-panel-header` / `.modal-panel-title` / `.modal-panel-description`
- `.modal-panel-body`
- `.modal-panel-footer` (flex-end, border-top, subtle bg tint)
- All CSS variables — no raw hex. Hard corners (`border-radius: 0`).

Import new SCSS in `modal-dispatch.tsx` (same pattern as `AgentNewBundleModal.scss`).

---

## 3. Identity Seed

### 3.1 Account model (DB)

Table `db_identity_accounts`:

| Column | Description |
|---|---|
| `id` | UUID (empty → server mints one) |
| `provider` | `"github"` \| `"aws"` \| `"anthropic"` \| … |
| `kind` | `"pat"` \| `"role"` \| `"api_key"` \| `"env_ref"` \| `"oauth"` |
| `display_name` | Human label shown in Trust Center |
| `secret_ref` | JSON — points backend to credential store |
| `context` | JSON — provider-specific extras (profile name, region, etc.) |
| `status` | `"unknown"` \| `"valid"` \| `"invalid"` |

Per-agent assignment stored as JSON blob in `db_agent_definitions.accounts`:
```json
{ "github": "<acct-uuid>", "aws": "<acct-uuid>" }
```

### 3.2 Agent → account mapping

| Agent | Display name | GitHub PAT secret | AWS profile | AWS account |
|---|---|---|---|---|
| agent1 | Agent1 GitHub | `services/infra → gh-token-agent1` | `Agent1` | `050544946291` |
| agent2 | Agent2 GitHub | `services/infra → gh-token-agent2` | `Agent2` | `050544946291` |
| agent3 | Agent3 GitHub | `services/infra → gh-token-agent3` | `Agent3` | `050544946291` |
| agent4 | Agent4 GitHub | `services/infra → gh-token-agent4` | `Agent4` | `050544946291` |
| agent5 | Agent5 GitHub | `services/infra → gh-token-agent5` | `Agent5` | `050544946291` |
| agentx | AgentX GitHub | `services/infra → gh-token-agentx` | `AgentX` | `050544946291` |
| agenty | AgentY GitHub | `services/infra → gh-token-agenty` | `AgentY` | `050544946291` |

**GitHub account `secret_ref`:**
```json
{
  "backend": "secrets_manager",
  "sm_path": "services/infra",
  "sm_json_path": "gh-token-agent1"
}
```

**AWS account `secret_ref`** (named profile — credentials already on disk):
```json
{
  "backend": "env_ref",
  "env_var": "AWS_PROFILE"
}
```
**AWS account `context`** (profile name + region):
```json
{
  "profile": "Agent1",
  "region": "us-east-1",
  "account_id": "050544946291"
}
```

### 3.3 GitHub App accounts (optional, Phase 2)

Each agent also has a GitHub App identity (`agent1-workflow`, etc.) for Layer 1 access. Credentials stored at `services/infra → agent-configs.<agentname>` (JSON with `GITHUB_APP_ID`, `GITHUB_APP_INSTALLATION_ID`) and `services/infra → <agentname>-workflow-key` (PEM). These are higher-privilege and can be added as a second `provider="github"` account per agent with `kind="role"` and `context: { "layer": 1, "app_id": "...", "installation_id": "..." }`.

### 3.4 Implementation options

**Option A — Frontend seed script (recommended for one-time)**

A TypeScript script at `scripts/seed-identities.ts` (run via `npx tsx scripts/seed-identities.ts`):

```typescript
// Pseudocode
const AGENTS = [
  { name: "agent1", githubSecretKey: "gh-token-agent1", awsProfile: "Agent1" },
  // ...agent2-5, agentx, agenty
];

for (const agent of AGENTS) {
  // 1. Find agent by name (GET agent definitions, filter by name)
  const agentDef = await rpc("listagents").then(r => r.find(a => a.name === agent.name));
  if (!agentDef) { console.warn(`Agent ${agent.name} not found — skipping`); continue; }

  // 2. Skip if already has github + aws assigned
  const existing = JSON.parse(agentDef.accounts || "{}");
  if (existing.github && existing.aws) { console.log(`${agent.name} already seeded`); continue; }

  // 3. Create GitHub account
  const ghAcct = await rpc("upsertidentityaccount", {
    id: "", provider: "github", kind: "pat",
    display_name: `${agent.name} GitHub`,
    secret_ref: JSON.stringify({ backend: "secrets_manager", sm_path: "services/infra", sm_json_path: agent.githubSecretKey }),
    context: "{}",
  });

  // 4. Create AWS account
  const awsAcct = await rpc("upsertidentityaccount", {
    id: "", provider: "aws", kind: "role",
    display_name: `${agent.name} AWS`,
    secret_ref: JSON.stringify({ backend: "env_ref", env_var: "AWS_PROFILE" }),
    context: JSON.stringify({ profile: agent.awsProfile, region: "us-east-1", account_id: "050544946291" }),
  });

  // 5. Assign to agent
  await rpc("updateagent", {
    ...agentDef,
    accounts: JSON.stringify({ github: ghAcct.id, aws: awsAcct.id }),
  });

  console.log(`✓ ${agent.name} seeded`);
}
```

**Option B — Rust seed extension**

Extend `agent_seed.rs` `SeedAgent` struct with `seed_accounts: Vec<SeedAccount>`. Called only when `seed_version` < threshold. More integrated but requires Rust rebuild.

**Option C — Trust Center one-shot button**

Add a "Seed from claw" button in the Trust Center (developer/admin-only, gated behind a feature flag or dev-mode check). Calls the same RPC sequence. Visible only when no agents have accounts assigned.

**Recommendation:** Option A for immediate use (run once, no rebuild). Option C for ongoing operational use (re-seedable from UI without CLI access).

### 3.5 RPC wire details

```typescript
// upsertidentityaccount
type UpsertIdentityAccountCommand = {
  id: string;            // empty = create new, uuid = update
  name: string;
  provider: string;
  kind: string;
  display_name: string;
  secret_ref: string;    // JSON string
  context: string;       // JSON string
};

// updateagent — pass full current agentDef spread + new accounts field
// accounts: JSON.stringify({ github: "<uuid>", aws: "<uuid>" })
```

---

## 4. Memory Seed

### 4.1 Architecture note

Native memory lives at:
```
~/.claude/projects/<sanitized-working-dir>/memory/<filename>.md
```
Path computed by `memory_dir_for_cwd(agent.working_directory)` — same algorithm in Rust and JS. **Agents must have `working_directory` set** for the path to resolve to something agent-specific. If empty, `agent:memory:write_file` returns an error.

### 4.2 Source files (a5af/claw)

Fetch from GitHub API: `gh api repos/a5af/claw/contents/<path>` (content is base64-encoded).

| claw path | Target filename | Applies to |
|---|---|---|
| `templates/CLAUDE.md` | `CLAUDE.md` | All agents |
| `templates/CRITICAL_RULES.md` | `CRITICAL_RULES.md` | All agents |
| `templates/host/CLAUDE.md` | `CLAUDE-host.md` | agentx, agenty |
| `templates/container/CLAUDE.md` | `CLAUDE-container.md` | agent1–5 |
| `templates/host/STARTUP_PROMPT.md` | `STARTUP_PROMPT.md` | agentx, agenty |
| `templates/container/STARTUP_PROMPT.md` | `STARTUP_PROMPT.md` | agent1–5 |
| `templates/CLAUDE_CONTAINER.md` | `CLAUDE_CONTAINER.md` | agent1–5 |
| `templates/skills/aws-setup.md` | `skills-aws-setup.md` | All agents |
| `templates/skills/github-layers.md` | `skills-github-layers.md` | All agents |
| `templates/skills/mcp-servers.md` | `skills-mcp-servers.md` | All agents |
| `templates/skills/git-workflow.md` | `skills-git-workflow.md` | All agents |

### 4.3 Frontmatter strategy

The claw templates have no YAML frontmatter. Prepend minimal frontmatter before writing:

```markdown
---
name: claude-identity
description: Agent identity, rules, and collaboration guidelines
metadata:
  type: user
---

<original file content>
```

Map claw files to frontmatter `type`:
- `CLAUDE.md`, `CLAUDE-host.md`, `CLAUDE-container.md`, `CLAUDE_CONTAINER.md` → `type: user`
- `CRITICAL_RULES.md` → `type: user`
- `STARTUP_PROMPT.md` → `type: reference`
- `skills-*.md` → `type: reference`

The `name` slug = filename stem (e.g. `CLAUDE` → `claude`, `CRITICAL_RULES` → `critical-rules`).

### 4.4 Implementation

Script at `scripts/seed-memories.ts` (run once via `npx tsx scripts/seed-memories.ts`):

```typescript
// Pseudocode
const CLAW_FILES = [
  { clawPath: "templates/CLAUDE.md", filename: "CLAUDE.md", agents: ALL, type: "user", name: "claude-identity" },
  { clawPath: "templates/CRITICAL_RULES.md", filename: "CRITICAL_RULES.md", agents: ALL, type: "user", name: "critical-rules" },
  { clawPath: "templates/host/CLAUDE.md", filename: "CLAUDE-host.md", agents: HOST, type: "user", name: "claude-host" },
  // ...
];

// Fetch all files from GitHub first
const contents = await Promise.all(
  CLAW_FILES.map(f => fetchClawFile(f.clawPath))
);

// Write per agent
for (const agentDef of agentDefs) {
  if (!agentDef.working_directory) {
    console.warn(`${agentDef.name}: no working_directory — skipping memory seed`);
    continue;
  }

  // First list existing files to skip already-seeded ones
  const existing = await rpc("agent:memory:list", { agent_id: agentDef.id });
  const existingNames = new Set(existing.files.map(f => f.filename));

  for (const [i, f] of CLAW_FILES.entries()) {
    if (!f.agents.includes(agentDef.name)) continue;
    if (existingNames.has(f.filename)) { console.log(`skip ${agentDef.name}/${f.filename}`); continue; }

    const body = addFrontmatter(contents[i], f);
    await rpc("agent:memory:write_file", {
      agent_id: agentDef.id,
      filename: f.filename,
      content: body,
    });
    console.log(`✓ ${agentDef.name}/${f.filename}`);
  }
}

function addFrontmatter(content: string, f: ClawFile): string {
  const slug = f.filename.replace(/\.md$/, "").toLowerCase().replace(/_/g, "-");
  return `---\nname: ${slug}\ndescription: ${f.description}\nmetadata:\n  type: ${f.type}\n---\n\n${content}`;
}
```

### 4.5 Working directory prerequisites

Memory seeding is a no-op for agents without `working_directory`. Before running the seed:
- agentx: set `working_directory` to `C:\Users\<user>\.claw\agentx-workspace` (or confirm it's already set)
- agenty: `C:\Users\<user>\.claw\agenty-workspace`
- agent1–5: set to the agent's container workspace root OR the host-side workspace mount (e.g. `C:\Users\<user>\.claw\workspaces\agent1`)

Memory is written to the **host filesystem** in all cases (via `memory_dir_for_cwd`) — it doesn't matter if the agent is a container or host agent from the path-resolution standpoint.

---

## 5. Phase Breakdown

### Phase T1 — Trust Center: Account detail modal (1 PR)
- Extract `AccountDetailPanel` from inline AccountsManager render
- Add `trust-center-account-detail` kind to modal-layer
- Remove `accounts-chooser-overlay` fixed div
- Wire `openModal()` call on row click
- SCSS for `AccountDetailPanel`

### Phase T2 — Trust Center: Account form + AgentMux connect (1 PR)
- Extract `AccountFormPanel` and `AgentMuxConnectPanel`
- Add `trust-center-account-form` and `trust-center-agentmux-connect` kinds
- Remove inline AccountForm expansion
- Wire modals

### Phase T3 — Trust Center: Identity + Memory detail modals (1 PR)
- Same pattern for IdentityManager and MemoryManager detail views
- Add `trust-center-identity-detail` and `trust-center-memory-detail` kinds

### Phase S1 — Identity seed script (1 script, no PR needed)
- `scripts/seed-identities.ts`
- Idempotent: check existing accounts before creating
- Confirm with user before write (`--dry-run` flag)
- Run: `npx tsx scripts/seed-identities.ts`

### Phase S2 — Memory seed script (1 script, no PR needed)
- `scripts/seed-memories.ts`
- Fetches from `a5af/claw` via `gh api`
- Adds frontmatter
- Idempotent: skips existing files
- Run: `npx tsx scripts/seed-memories.ts`

### Phase S3 — Trust Center "Seed from claw" button (optional, 1 PR)
- Surfaced in Trust Center dev panel or admin section
- Runs identity + memory seed logic inline (same RPCs)
- Gated: visible only in dev mode or when env `AGENTMUX_SEED_UI=1`

---

## 6. Open Questions

1. **Trust Center tab panels always mounted** — toggling via `is-hidden` means all three managers are mounted simultaneously. Should the modal refactor also switch them to lazy-mount (unmount on hide, remount on show)? Lazy-mount reduces DOM but loses unsaved form state. Recommend: keep always-mounted for now, flag as a follow-up.

2. **GitHub App accounts** (Phase 2 of identity seed) — do we want Layer 1 (App) accounts seeded alongside Layer 2 (PAT) accounts? Adds complexity: need to pull `GITHUB_APP_ID` and `GITHUB_APP_INSTALLATION_ID` from `services/infra → agent-configs.<agentname>`. Defer unless needed.

3. **agenty PAT path** — `gh-token-agenty` doesn't appear in the confirmed Secrets Manager key list (only `gh-token-agent1` through `gh-token-agent5` and `gh-token-agentx` confirmed). Verify before seeding.

4. **Memory working_directory** — agent1–5 may be container agents with a Linux working_directory (e.g. `/workspace`). The memory path computed on the host would be `~/.claude/projects/-workspace/memory/` which may conflict if multiple container agents use `/workspace`. Clarify whether each container agent has a distinct host-side workspace path or a container-internal Linux path.

5. **`--dry-run` flag** — seed scripts should print what they would do without writing. Implement for safety.

---

## 7. File Index

| File | Role |
|---|---|
| `frontend/app/modals/bundle-manager-modal.tsx` | Trust Center root — contains AccountsManager, IdentityManager, MemoryManager |
| `frontend/app/element/modal-layer.ts` | `ModalLayerRequest` union — add new kinds here |
| `frontend/app/element/modal-dispatch.tsx` | `requestLabel` + `renderRequest` switch — add cases here |
| `frontend/app/modals/trust-center/` | New dir for extracted panel components |
| `agentmux-srv/src/backend/storage/identities.rs` | Identity CRUD (Rust) |
| `agentmux-srv/src/server/native_memory_handlers.rs` | Memory RPCs (Rust, complete) |
| `scripts/seed-identities.ts` | One-time identity seed (new) |
| `scripts/seed-memories.ts` | One-time memory seed (new) |
| `a5af/claw` | Source for memory template `.md` files |
