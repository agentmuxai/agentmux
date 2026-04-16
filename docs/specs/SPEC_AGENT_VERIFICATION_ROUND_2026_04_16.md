# SPEC: Agent Startup Verification Round

**Date:** 2026-04-16
**Status:** Draft
**Related:** SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md, a5af/claw `/startup` skill

---

## Problem

When an agent session starts, the startup sequence sends context (identity,
accounts, peers) but doesn't verify that the agent's tools actually work.
In claw, the `/startup` skill runs a verification round that checks every
tool and reports a status table. AgentMux should do the same — without
requiring claw to be installed.

Without verification:
- Agent assumes GitHub auth works, then fails mid-PR
- Dev-tools aren't installed, agent discovers this during a deploy
- MCP servers aren't connected, inter-agent messaging silently fails
- AWS credentials are expired, discovered only when needed

---

## Goal

After the startup context message, the agent automatically runs a
verification round that:

1. Confirms all tools are functional (not just configured)
2. Installs missing dev-tools from `@a5af/dev-tools`
3. Reports a status table visible in the conversation
4. Continues only after all critical checks pass

---

## Design

### 1. Verification as a Startup Instruction

The verification round is driven by the **startup content** in ForgeContent
(`content_type: "startup"`). Instead of hardcoding checks in AgentMux, the
startup instructions tell the agent what to verify. This keeps the checks
configurable per agent.

### 2. Standard Startup Content Template

When seeding agents or when the user doesn't provide custom startup content,
use this default template (stored in ForgeContent `startup`):

```markdown
## Verification Round

Run these checks and report results in a table. Fix any failures before proceeding.

### Identity
- Run `gh auth status` — confirm GitHub identity
- Run `aws sts get-caller-identity` — confirm AWS profile (if AWS account assigned)
- Verify git identity: `git config user.name` and `git config user.email`

### Dev Tools
Install if missing, then verify:
```bash
npm list -g @a5af/secrets @a5af/deploy-cli @a5af/database-cli @a5af/api-testing @a5af/file-tools @a5af/e2e-cli @a5af/reagent-cli @a5af/workspace-health 2>/dev/null | grep -E "@a5af/"
```

If any are missing:
```bash
npm install -g @a5af/secrets @a5af/deploy-cli @a5af/database-cli @a5af/api-testing @a5af/file-tools @a5af/e2e-cli @a5af/reagent-cli @a5af/workspace-health
```

Then verify each works:
```bash
secrets --version
deploy --version
```

### MCP Servers
- Check AgentBus connectivity: list available agents
- Check any provider-specific MCP servers

### Report Format

| Check | Status | Details |
|-------|--------|---------|
| GitHub | OK/FAIL | username, scopes |
| AWS | OK/FAIL/SKIP | profile, account ID |
| Git Identity | OK/FAIL | name, email |
| Dev Tools | OK/FAIL | installed count/total |
| MCP AgentBus | OK/FAIL | agent count |

### After Verification

If all critical checks pass, report "Verification complete" and wait for
user instructions. If any FAIL, attempt to fix automatically. If unfixable,
report what's broken and ask the user.
```

### 3. Seeding the Default Startup Content

The forge seed manifest already supports content blobs. Add a `startup`
content type to the default agents:

```json
{
    "id": "agentx",
    "content": {
        "env": "AGENT_NAME=agentx\nAGENTMUX_AGENT_ID=AgentX",
        "startup": "## Verification Round\n\nRun these checks..."
    }
}
```

### 4. Dev-Tools Installation

The startup instructions tell the agent to install `@a5af/dev-tools`
packages if missing. This requires:

- `npm` on PATH (already required for provider CLIs)
- GitHub Packages auth configured in `.npmrc` (already set up by
  `GH_CONFIG_DIR` env var or global npmrc)
- `@a5af` scope registry pointing to `npm.pkg.github.com`

The agent handles this via bash commands in the startup instructions —
no AgentMux code changes needed.

### 5. Per-Agent Customization

Different agents may need different checks:

- **Host agents (AgentX):** Full stack — GitHub, AWS, dev-tools, MCP
- **Container agents (Agent1-3):** Subset — GitHub, workspace health
- **New user agents:** Minimal — just GitHub and git identity

Users customize by editing the `startup` content in the Forge settings
panel (the same panel where they edit soul, instructions, env).

### 6. Re-verification

The `/startup` slash command (already exists in claw) should be available
as a ForgeSkill so the user can re-run verification at any time:

```json
{
    "name": "Startup Verification",
    "trigger": "startup",
    "skill_type": "prompt",
    "description": "Re-run tool verification checks",
    "content": "Run the verification round from your startup instructions and report the status table."
}
```

---

## Implementation Plan

### PR 1: Add startup content to seed manifest

1. Add `"startup"` to the content blobs in `forge-seed.json` for all agents
2. Content is the standard verification template above
3. The existing `buildStartupPayload` already fetches and includes
   `ForgeContent("startup")` — no code changes needed

### PR 2: Add `/startup` skill to seed manifest

1. Add a `startup` skill to each agent in `forge-seed.json`
2. Trigger: `startup`, content: re-run verification prompt

### PR 3: Ensure .npmrc for @a5af packages (optional)

1. During agent launch, check if `.npmrc` has `@a5af` scope configured
2. If not, write it using the agent's GitHub token
3. This ensures `npm install -g @a5af/*` works during verification

---

## Non-Goals

- **Hardcoding verification logic in Rust/TypeScript.** The verification
  is driven by the startup content (Markdown instructions). The agent
  executes it using its normal tool-calling capabilities.
- **Blocking the session on verification.** The agent reports results and
  continues. The user can interrupt at any time.
- **Replacing claw.** For agents deployed via claw, claw's `/startup`
  skill takes precedence. This spec covers agents launched directly
  through AgentMux without claw.
