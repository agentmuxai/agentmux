// Generates forge-seed.json with proper startup content
const HOST_STARTUP = `## Verification Round

Run each check below. Fix failures before proceeding. Report results in a table.

### 1. Identity
\`\`\`bash
gh auth status
git config user.name && git config user.email
\`\`\`
If \`gh auth status\` fails, run \`gh auth login\`.

### 2. Dev Tools
Install if missing, then verify:
\`\`\`bash
npm list -g @a5af/secrets @a5af/deploy-cli @a5af/database-cli @a5af/api-testing @a5af/file-tools @a5af/e2e-cli @a5af/reagent-cli @a5af/workspace-health 2>/dev/null | grep '@a5af/' || npm install -g @a5af/secrets @a5af/deploy-cli @a5af/database-cli @a5af/api-testing @a5af/file-tools @a5af/e2e-cli @a5af/reagent-cli @a5af/workspace-health
\`\`\`
Verify key tools:
\`\`\`bash
secrets --version && deploy --version && reagent --version
\`\`\`

### 3. Secrets Access
\`\`\`bash
secrets health services/infra services/prod
\`\`\`

### 4. MCP Servers
Check AgentBus connectivity — list available peer agents.

### Report

| Check | Status | Details |
|-------|--------|--------|
| GitHub | OK/FAIL | username |
| Git Identity | OK/FAIL | name, email |
| Dev Tools | OK/FAIL | count installed |
| Secrets | OK/FAIL | accessible secrets |
| MCP AgentBus | OK/FAIL | peer count |

If all pass: "Verification complete — ready to work."
If any FAIL: attempt fix. If unfixable, report and ask.`;

const CONTAINER_STARTUP = `## Verification Round

Run each check below. Fix failures before proceeding. Report results in a table.

### 1. Identity
\`\`\`bash
gh auth status
git config user.name && git config user.email
\`\`\`

### 2. Dev Tools
Install if missing:
\`\`\`bash
npm list -g @a5af/secrets @a5af/file-tools @a5af/reagent-cli 2>/dev/null | grep '@a5af/' || npm install -g @a5af/secrets @a5af/file-tools @a5af/reagent-cli
\`\`\`

### 3. MCP Servers
Check AgentBus connectivity — list available peer agents.

### Report

| Check | Status | Details |
|-------|--------|--------|
| GitHub | OK/FAIL | username |
| Git Identity | OK/FAIL | name, email |
| Dev Tools | OK/FAIL | count installed |
| MCP AgentBus | OK/FAIL | peer count |

If all pass: "Verification complete — ready to work."
If any FAIL: attempt fix. If unfixable, report and ask.`;

const SKILL = {
    name: "Startup Verification",
    trigger: "startup",
    skill_type: "prompt",
    description: "Re-run tool verification checks and report status table",
    content: "Run the verification round from your startup instructions. Check identity (gh, git), dev-tools (@a5af/* packages — install if missing), secrets access, and MCP servers. Report a status table with OK/FAIL for each check. Fix any failures automatically if possible."
};

const CONTAINER_SKILL = { ...SKILL, content: SKILL.content.replace(", secrets access,", ",") };

const manifest = {
    version: 4,
    agents: [
        { id: "agentx", name: "AgentX", icon: "\ud83d\udd34", description: "Primary coding agent", working_directory: "~/.agentmux/agents/agentx", shell: "pwsh", agent_bus_id: "agentx", content: { env: "AGENT_NAME=agentx\nAGENTMUX_AGENT_ID=AgentX", startup: HOST_STARTUP }, skills: [SKILL] },
        { id: "agenty", name: "AgentY", icon: "\ud83d\udfe1", description: "Secondary coding agent", working_directory: "~/.agentmux/agents/agenty", shell: "pwsh", agent_bus_id: "agenty", content: { env: "AGENT_NAME=agenty\nAGENTMUX_AGENT_ID=AgentY", startup: HOST_STARTUP }, skills: [SKILL] },
        { id: "agentz", name: "AgentZ", icon: "\ud83d\udd35", description: "Tertiary coding agent", working_directory: "~/.agentmux/agents/agentz", shell: "pwsh", agent_bus_id: "agentz", content: { env: "AGENT_NAME=agentz\nAGENTMUX_AGENT_ID=AgentZ", startup: HOST_STARTUP }, skills: [SKILL] },
        { id: "agent1", name: "Agent1", icon: "\ud83d\udfe2", description: "Sandboxed coding agent", working_directory: "/workspace", shell: "bash", agent_bus_id: "agent1", restart_on_crash: true, content: { env: "AGENT_NAME=agent1\nAGENTMUX_AGENT_ID=Agent1", startup: CONTAINER_STARTUP }, skills: [CONTAINER_SKILL] },
        { id: "agent2", name: "Agent2", icon: "\ud83d\udfe0", description: "Sandboxed coding agent", working_directory: "/workspace", shell: "bash", agent_bus_id: "agent2", restart_on_crash: true, content: { env: "AGENT_NAME=agent2\nAGENTMUX_AGENT_ID=Agent2", startup: CONTAINER_STARTUP }, skills: [CONTAINER_SKILL] },
        { id: "agent3", name: "Agent3", icon: "\ud83d\udfe3", description: "Sandboxed coding agent", working_directory: "/workspace", shell: "bash", agent_bus_id: "agent3", restart_on_crash: true, content: { env: "AGENT_NAME=agent3\nAGENTMUX_AGENT_ID=Agent3", startup: CONTAINER_STARTUP }, skills: [CONTAINER_SKILL] },
    ]
};

process.stdout.write(JSON.stringify(manifest, null, 2) + "\n");
