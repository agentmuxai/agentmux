# Spec: Forge Agent Identity — GitHub + AWS + Git

**Date:** 2026-04-13
**Status:** Draft — ready for discussion
**Related:**
- `a5af/claw` — PowerShell launcher with per-agent identity (GitHub + AWS + MCP); we're porting its identity model natively into AgentMux's Forge.
- `agentmux-srv/src/server/app_api.rs:142-200` — where agent env vars are currently set on spawn.
- `agentmux-srv/src/backend/storage/wstore.rs:399-428` — `ForgeAgent` struct.

---

## 1. What exists today

AgentMux already does **partial** per-agent identity isolation. Current state:

### 1.1 ForgeAgent fields (wstore.rs:399-428)

```rust
pub struct ForgeAgent {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub provider: String,                    // claude | codex | gemini
    pub description: String,
    pub working_directory: String,           // defaults to ~/.agentmux/agents/<slug>
    pub shell: String,
    pub provider_flags: String,
    pub auto_start: i64,
    pub restart_on_crash: i64,
    pub idle_timeout_minutes: i64,
    pub created_at: i64,
    pub agent_type: String,                  // standalone | host | container
    pub environment: String,
    pub agent_bus_id: String,
    pub is_seeded: i64,
}
```

**Zero identity fields.** No GitHub user, no AWS profile, no git author, no SSH key path. Identity is implicit from whatever the process running on the machine has in its global state.

### 1.2 Env vars set at spawn time (app_api.rs:142-200)

When `agent.open` spawns a block, it sets:

```rust
// Auth dir — provider CLI stores its own config here (Claude, Codex, etc.)
env_vars.insert(
    provider.auth_config_dir_env_var,  // e.g. "CLAUDE_CONFIG_DIR"
    json!(format!("{home}/.agentmux/config/auth/{auth_dir_name}")),
);

// GitHub CLI — per-slug isolation
env_vars.insert("GH_CONFIG_DIR".to_string(),
    json!(format!("~/.agentmux/config/gh-{agent_slug}")));

// Agent identity (used by shell integration prompt + pane title)
env_vars.insert("AGENTMUX_AGENT_ID".to_string(), json!(&agent.name));
```

Plus `provider.unset_env` blanks out a handful of inherited env vars, and `provider.auth_extra_env` adds any provider-specific keys.

### 1.3 What's working

- **Claude/Codex/Gemini auth** per provider. Each provider CLI stores its tokens under `~/.agentmux/config/auth/<provider>/` in isolation from the user's own `~/.claude/`.
- **GitHub CLI** per agent slug. `gh auth login` inside an agent pane writes to `~/.agentmux/config/gh-<slug>/hosts.yml` and does not pollute the user's primary `gh` state. `gh` commands issued from that pane use the slug-isolated token.
- **Shell integration** sees `AGENTMUX_AGENT_ID` and uses it for prompt coloring + pane title routing.

### 1.4 What's missing (the gap)

| Identity surface | Current state | Gap |
|---|---|---|
| GitHub CLI (`gh`) | ✅ per-slug via `GH_CONFIG_DIR` | OK |
| GitHub PAT / Fine-grained token | Implicit from `gh auth` | No way to configure a distinct PAT per agent without running `gh auth login` manually inside each pane |
| Git author / committer (`user.name`, `user.email`) | ❌ inherited from host `~/.gitconfig` | Agent commits as whoever the host's git user is. Two agents on the same host commit as the same person — which is wrong. |
| Git SSH key (for `git@github.com`) | ❌ inherited | Same issue — all agents use the same key |
| AWS profile | ❌ not set | All agents get whatever `~/.aws/credentials` default profile is, or `$AWS_PROFILE` from the host environment |
| AWS credentials file path | ❌ not set | No isolation — agents share `~/.aws/credentials` |
| NPM auth (`~/.npmrc`) | ❌ inherited | Agents publish to the host user's npm registry context |
| Docker registry (`~/.docker/config.json`) | ❌ inherited | Same |
| SSH config (`~/.ssh/config`) | ❌ inherited | Agents can SSH to anywhere the host user can |

**The Forge UI has no surface for any of this.** Even the existing `GH_CONFIG_DIR` isolation is invisible to the user — they just discover it works when `gh auth login` inside the pane doesn't clobber their host config.

### 1.5 What `a5af/claw` does

From the claw README §"Per-Agent Configuration":

> Each agent gets isolated:
> - **GitHub CLI**: `~/.config/gh-<agent>/`
> - **AWS Profile**: `Agent1`, `AgentX`, etc.
> - **MCP Servers**: Per-workspace `.mcp.json`

> **GitHub Access (3-Layer)**
> 1. Layer 1 (PRIMARY): GitHub App via MCP
> 2. Layer 2 (SECONDARY): Agent PAT via gh CLI
> 3. Layer 3 (ADMIN): a5af admin PAT for privileged ops

Claw's model:

- GitHub: one `gh` config dir per agent (**same pattern AgentMux already uses**). Plus MCP-based GitHub App access as the preferred path, PAT-based `gh` as fallback, admin PAT for privileged ops.
- AWS: named profiles in a shared `~/.aws/credentials`. Agent1's spawn script exports `AWS_PROFILE=Agent1`. No isolation of the credentials *file* — just profile selection.
- MCP: per-workspace `.mcp.json` (the MCP server config file loaded by Claude CLI at startup).

Claw's limitation: it's a PowerShell launcher, so the identity config is outside the agent record itself — you configure it via env files and Docker mounts. There's no central "this is agent foo's identity" store.

AgentMux has an advantage: the Forge agent record already exists as a database row. We can put identity *on the record*, pass it to the spawn path, and expose a UI for editing. That's strictly more than what claw does.

---

## 2. Proposed identity model

### 2.1 Extend `ForgeAgent` with an `identity` struct

```rust
/// Identity credentials injected into the agent's subprocess environment.
///
/// All fields are optional — unset means "inherit from host". For true
/// isolation, set them explicitly on the Forge agent record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForgeAgentIdentity {
    // ─── Git ─────────────────────────────────────────────────
    /// Sets GIT_AUTHOR_NAME + GIT_COMMITTER_NAME at spawn time.
    /// Falls back to host git config when unset.
    #[serde(default)]
    pub git_author_name: String,
    /// Sets GIT_AUTHOR_EMAIL + GIT_COMMITTER_EMAIL at spawn time.
    #[serde(default)]
    pub git_author_email: String,
    /// Absolute path to an SSH private key used for `git@github.com:...`
    /// remotes. Sets GIT_SSH_COMMAND="ssh -i <path> -o IdentitiesOnly=yes".
    #[serde(default)]
    pub git_ssh_key_path: String,

    // ─── GitHub ──────────────────────────────────────────────
    /// Overrides the auto-generated `~/.agentmux/config/gh-<slug>` path.
    /// Leave empty to use the default (which is what we do today).
    #[serde(default)]
    pub gh_config_dir: String,
    /// If set, populates the agent's gh config with this PAT at spawn
    /// time (written to <gh_config_dir>/hosts.yml). Mutually exclusive
    /// with gh_app_id/installation_id.
    #[serde(default)]
    pub github_pat: String,
    /// GitHub App installation ID. When set, agent uses a GitHub App
    /// token minted at spawn time (MCP-backed or direct API call).
    /// Preferred over PAT per claw's 3-layer model.
    #[serde(default)]
    pub github_app_installation_id: String,

    // ─── AWS ─────────────────────────────────────────────────
    /// Named profile in the shared credentials file to activate.
    /// Simple option — matches claw's pattern. Uses the host's
    /// ~/.aws/credentials but selects this profile via $AWS_PROFILE.
    #[serde(default)]
    pub aws_profile: String,
    /// Path to an isolated credentials file. When set, overrides
    /// AWS_SHARED_CREDENTIALS_FILE so the agent reads from a
    /// per-agent credentials file. Defaults to
    /// ~/.agentmux/config/aws-<slug>/credentials if this is empty
    /// AND any other AWS field is set, for true isolation.
    #[serde(default)]
    pub aws_credentials_file: String,
    /// Same for the config file (region, role_arn, etc.).
    #[serde(default)]
    pub aws_config_file: String,
    /// Optional explicit region override.
    #[serde(default)]
    pub aws_region: String,

    // ─── NPM / Node ──────────────────────────────────────────
    /// Path to a per-agent .npmrc. Sets NPM_CONFIG_USERCONFIG.
    #[serde(default)]
    pub npmrc_path: String,

    // ─── Extra env ───────────────────────────────────────────
    /// Free-form env vars appended after the identity-derived ones.
    /// Values here take precedence (set last in the HashMap).
    /// Format: "KEY1=value1\nKEY2=value2".
    #[serde(default)]
    pub extra_env: String,
}
```

Add as a new field on `ForgeAgent`:

```rust
pub struct ForgeAgent {
    // … existing fields
    #[serde(default)]
    pub identity: ForgeAgentIdentity,
}
```

`#[serde(default)]` means existing agent records migrate silently — they just get an all-empty identity struct. No backend migration needed beyond "next read deserializes with the new field."

### 2.2 Env var injection at spawn time

Extend the env-building block in `app_api.rs:152-169` with an identity section. Rough shape (I'll reference the real layout when implementing):

```rust
let identity = &agent.identity;

// Git author/committer — falls through to host gitconfig if unset
if !identity.git_author_name.is_empty() {
    env_vars.insert("GIT_AUTHOR_NAME".into(),   json!(&identity.git_author_name));
    env_vars.insert("GIT_COMMITTER_NAME".into(), json!(&identity.git_author_name));
}
if !identity.git_author_email.is_empty() {
    env_vars.insert("GIT_AUTHOR_EMAIL".into(),   json!(&identity.git_author_email));
    env_vars.insert("GIT_COMMITTER_EMAIL".into(), json!(&identity.git_author_email));
}

// Git SSH — bind a specific private key for git@github.com remotes.
// IdentitiesOnly=yes prevents ssh from falling back to the user's default
// keys if this one is rejected.
if !identity.git_ssh_key_path.is_empty() {
    env_vars.insert(
        "GIT_SSH_COMMAND".into(),
        json!(format!(
            "ssh -i {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new",
            identity.git_ssh_key_path
        )),
    );
}

// GitHub CLI — default to ~/.agentmux/config/gh-<slug>, allow override
let gh_dir = if identity.gh_config_dir.is_empty() {
    format!("~/.agentmux/config/gh-{}", agent_slug)
} else {
    identity.gh_config_dir.clone()
};
env_vars.insert("GH_CONFIG_DIR".into(), json!(&gh_dir));

// GitHub PAT — write into gh_dir/hosts.yml at spawn time so the first
// `gh` command works without a prompt. Skipped if github_app_installation_id
// is set (takes precedence).
if !identity.github_app_installation_id.is_empty() {
    // Mint an installation token via the App private key stored in
    // ~/.agentmux/config/github-app.pem (separate file, not in the
    // agent record). Set GH_TOKEN env var with the short-lived token.
    // Refresh cadence: install a background worker similar to
    // SessionArchiver that rotates tokens on expiry.
    env_vars.insert("GH_TOKEN".into(), json!(mint_app_token(&identity.github_app_installation_id)?));
} else if !identity.github_pat.is_empty() {
    // Write to <gh_dir>/hosts.yml at spawn time (idempotent — only if
    // the file is missing or the token has changed).
    ensure_gh_hosts_yml(&gh_dir, &identity.github_pat)?;
    env_vars.insert("GH_TOKEN".into(), json!(&identity.github_pat));
}

// AWS — three-layer cascade matching what the CLI expects
if !identity.aws_profile.is_empty() {
    env_vars.insert("AWS_PROFILE".into(), json!(&identity.aws_profile));
}
if !identity.aws_credentials_file.is_empty() {
    env_vars.insert(
        "AWS_SHARED_CREDENTIALS_FILE".into(),
        json!(&identity.aws_credentials_file),
    );
} else if aws_identity_any_set(&identity) {
    // If the user configured ANY AWS field without specifying a file,
    // default to a per-slug isolated credentials file.
    env_vars.insert(
        "AWS_SHARED_CREDENTIALS_FILE".into(),
        json!(format!("~/.agentmux/config/aws-{}/credentials", agent_slug)),
    );
}
if !identity.aws_config_file.is_empty() {
    env_vars.insert("AWS_CONFIG_FILE".into(), json!(&identity.aws_config_file));
}
if !identity.aws_region.is_empty() {
    env_vars.insert("AWS_REGION".into(), json!(&identity.aws_region));
    env_vars.insert("AWS_DEFAULT_REGION".into(), json!(&identity.aws_region));
}

// NPM
if !identity.npmrc_path.is_empty() {
    env_vars.insert("NPM_CONFIG_USERCONFIG".into(), json!(&identity.npmrc_path));
}

// Free-form extras (set LAST so they win over identity-derived defaults)
for line in identity.extra_env.lines() {
    if let Some((k, v)) = line.split_once('=') {
        env_vars.insert(k.trim().into(), json!(v.trim()));
    }
}
```

### 2.3 Frontend — identity editor

Add a new section to the Forge agent editor UI called **"Identity"** with:

| Field | Input | Validation |
|---|---|---|
| Git author name | text | any |
| Git author email | text | regex: `/.+@.+\..+/` |
| SSH key path | file picker | must exist + mode 600 warning |
| `gh` config dir | text (advanced, hidden by default) | path |
| GitHub PAT | password field | `ghp_*` / `github_pat_*` prefix |
| GitHub App installation ID | text | numeric |
| AWS profile | text | alphanum / `-` / `_` |
| AWS credentials file | file picker | any |
| AWS config file | file picker | any |
| AWS region | dropdown (us-east-1 etc.) or free text | known regions |
| `.npmrc` path | file picker | any |
| Extra env vars | textarea (KEY=VALUE per line) | each line must be `KEY=VALUE` format |

UI shows a "test identity" button that spawns a temporary block, runs:
```bash
echo "git: $(git config --global user.name) <$(git config --global user.email)>"
echo "gh: $(gh api user --jq .login 2>/dev/null || echo 'not authenticated')"
echo "aws: $(aws sts get-caller-identity --query Arn --output text 2>/dev/null || echo 'not configured')"
```
and shows the output. Read-only, ~3 seconds, cleaned up automatically.

### 2.4 Three-layer GitHub model (later phase)

Claw's 3-layer model maps cleanly onto a second phase of this work:

**Layer 1 — GitHub App via MCP** (highest trust, preferred)
- User configures a GitHub App ID + private key PEM in `~/.agentmux/config/github-app.pem`
- Each agent stores its `github_app_installation_id`
- At spawn time, agentmux-srv mints a short-lived installation token (10 min expiry) and sets `GH_TOKEN`
- Background worker rotates tokens every 8 minutes for running agents
- `mcp.json` can also reference the GitHub App for MCP-native tools

**Layer 2 — Agent PAT** (fallback, per-agent)
- `github_pat` field on the identity struct
- Lower trust, longer-lived
- Used when GitHub App isn't configured

**Layer 3 — Admin PAT** (privileged ops only)
- Stored separately in `~/.agentmux/config/admin-token` (NOT on any agent)
- Used only by a narrow set of RPCs that need org-admin operations (e.g. repo create)
- Never exposed to agent processes

Layer 1 is out of scope for the initial PR. Start with Layers 2 + 3 (PAT + file-stored admin token) and add App-minting later. All three coexist once implemented — agents pick highest-available layer via precedence.

---

## 3. Rollout plan

**Three PRs.** Each is shippable standalone.

### PR 1 — Backend identity struct + env injection

- `agentmux-srv/src/backend/storage/wstore.rs` — add `ForgeAgentIdentity` struct, field on `ForgeAgent`
- `agentmux-srv/src/server/app_api.rs` — extend env-var block to read from `agent.identity`
- `agentmux-srv/src/backend/rpc_types.rs` — `CommandCreateForgeAgentData` + `CommandUpdateForgeAgentData` gain an optional `identity` field
- `frontend/types/gotypes.d.ts` — TS types for the new struct
- **No UI yet.** Identity can be set by calling the RPC directly (or editing the DB). Existing agents auto-migrate because of `#[serde(default)]`.
- **Verify** by creating a Forge agent with `git_author_name=Bob, git_author_email=bob@example.com`, opening its pane, running `git config --global user.name` — should print `Bob`. Run `env | grep GIT_AUTHOR` — should be set.
- **Est:** 150 lines backend + 50 lines types, 1 hour.

### PR 2 — Frontend identity editor in Forge UI

- Extend the Forge agent editor to show an "Identity" collapsible section with the fields from §2.3
- Add the "test identity" button + temporary-spawn-and-read flow
- No backend changes; uses the RPC surface from PR 1
- **Est:** 200-300 lines SolidJS + some CSS, 2-3 hours. Needs reagent review because it touches form state.

### PR 3 — GitHub App minting (layer 1) and AWS directory isolation defaults

- Add a small in-process GitHub App token-mint service (fetches ~/.agentmux/config/github-app.pem, generates JWT, exchanges for installation token, caches with 8-min TTL)
- Background worker that rotates tokens for spawned agents still running
- Default `AWS_SHARED_CREDENTIALS_FILE` to a per-slug path when any AWS field is set but file isn't explicitly named
- Populate `~/.agentmux/config/aws-<slug>/credentials` with a single `[default]` section pointing at the profile the user selected, so `aws` commands work even without explicit `--profile`
- **Est:** 200 lines Rust, 3-4 hours. Needs JWT library (`jsonwebtoken` crate), minor dep addition.

### Out of scope for this spec (separate specs if needed)

- **MCP server config** for GitHub App — `.mcp.json` generation per agent. Claw does this; we don't have MCP plumbing in the Forge workflow yet.
- **Secret rotation / revocation UI.** "Delete this agent's GitHub PAT" button etc.
- **Hardware-backed key storage.** Per-agent credentials currently land on disk in plaintext. Worth a separate spec about OS keyring integration (`keyring-rs` on Rust) for sensitive fields.
- **Container-mode identity.** When the agent runs in a Docker container (`agent_type: "container"`), the env vars need to be passed via `-e` to `docker run`. Currently agentmux-srv spawns local processes; container support is a separate path that needs its own env-injection story.
- **Shared secrets across agents.** Sometimes two agents should share a secret (e.g. same AWS role). Currently each agent would have its own copy; a "shared credential pool" concept is future work.

---

## 4. Principles

1. **Identity is optional, inheritance is the default.** An agent with no identity fields set inherits the host user's state. This is compatible with current behavior.
2. **Per-slug file paths are the escape hatch.** When the user wants true isolation, the defaults route everything to `~/.agentmux/config/<tool>-<slug>/`. Same pattern we already use for `GH_CONFIG_DIR`.
3. **Env vars are the mechanism.** Every identity surface is configurable via a well-known env var that the target CLI respects: `GIT_AUTHOR_NAME`, `GH_CONFIG_DIR`, `AWS_PROFILE`, `AWS_SHARED_CREDENTIALS_FILE`, etc. No hooking into the CLI's config file format.
4. **Identity lives on the Forge record, not in env files or Dockerfiles.** This is the AgentMux advantage over claw — identity is data, editable via UI, auditable via the Forge list.
5. **Extra env as a free-form escape hatch.** When we miss a surface (a new tool's env var, a custom internal config), the `extra_env` textarea lets the user ship without waiting for a new field on the struct.
6. **Secrets never leave the backend process before spawn time.** The frontend sends them over WebSocket RPC (authenticated to the user's local server), they're stored in the local SQLite DB, and injected into child-process env at spawn. No browser-side secret handling beyond the input field.

---

## 5. Compatibility with `a5af/claw`

We're not porting claw itself — we're porting its *model*. The mapping:

| Claw concept | AgentMux equivalent |
|---|---|
| `~/.claw/workspaces/<agent>/` | `~/.agentmux/agents/<slug>/` (already exists) |
| Per-agent `gh-<agent>` config | `GH_CONFIG_DIR=~/.agentmux/config/gh-<slug>` (already exists) |
| Named AWS profile `Agent1` | `identity.aws_profile = "Agent1"` (new) |
| Per-workspace `.mcp.json` | *future PR* — see §3 out of scope |
| Claw's PowerShell launcher | agentmux-srv's `agent.open` RPC + spawn path |
| Claw's 3-layer GitHub access | PR 3 in our rollout |

**Interop:** if the user has claw installed and uses its `~/.config/gh-<agent>` paths, we can make AgentMux point at those same paths via the `gh_config_dir` override field. No migration needed — `gh_config_dir` is text. Same for AWS files.

**Non-interop:** claw's PowerShell-based `claw container agent1` Docker launcher is parallel, not replaced. Users who want containerized agents still run claw; users who want native agents on the host use AgentMux's Forge + spawn path. Both can coexist on the same machine.

---

## 6. Action items

1. **Decide:** ship as PR 1 only (minimum viable), or bundle PRs 1+2 (backend + UI together). PR 3 (GitHub App minting) should probably wait until someone actually asks for it.
2. **Decide:** do we put the identity section in the existing Forge agent editor, or behind a new "Identity" tab?
3. **Approve the field list in §2.1.** Anything missing (e.g. GCP `GOOGLE_APPLICATION_CREDENTIALS` path, Azure `AZURE_CONFIG_DIR`, Hugging Face `HF_TOKEN`)?
4. **Approve the env var precedence:** identity-derived first, then `extra_env` textarea overrides. Correct?
5. **Decide:** do we need the "test identity" spawn-temporary-pane button in PR 2, or is it PR 2.5 polish?

Once approved, I start on PR 1.
