// Generates forge-seed.json — one seeded definition per CLI in the
// catalog (frontend/app/view/agent/defaults/cli-catalog.ts). Each row
// is a template; a running instance is created when the user picks a
// card + supplies a name via AgentLaunchModal.
//
// See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md §5.
//
// Manifest version 5 (2026-04-23): replaces the AgentX/Y/Z + Agent1/2/3
// layout with per-CLI definitions. The re-seed engine in
// `agentmux-srv/src/backend/forge_seed.rs` deletes old seeded rows
// and inserts the new set on version bump — user customisations to
// the old rows are lost. This is intentional: the old layout
// conflated 6 agents into 3 providers, which no longer matches the
// "one card per CLI" model the picker now shows.

const STARTUP = `## Verification Round

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

const SKILL = {
    name: "Startup Verification",
    trigger: "startup",
    skill_type: "prompt",
    description: "Re-run tool verification checks and report status table",
    content:
        "Run the verification round from your startup instructions. Check identity (gh, git), dev-tools (@a5af/* packages — install if missing), secrets access, and MCP servers. Report a status table with OK/FAIL for each check. Fix any failures automatically if possible.",
};

// Catalog-aligned seed. Keep in lockstep with
// `frontend/app/view/agent/defaults/cli-catalog.ts`.
// `working_directory` intentionally empty: `agent-model.ts` resolves
// it at launch-time (portable vs installed build) and when a user
// launches via AgentLaunchModal it's overridden with a fresh
// `<slug>-<YYYYMMDD-HHMMSS>` path.
const CLI_DEFS = [
    {
        id: "claude",
        name: "Claude",
        provider: "claude",
        icon: "\u2716",                  // ✖
        description: "Anthropic's coding agent",
        bus: "claude",
    },
    {
        id: "codex",
        name: "Codex",
        provider: "codex",
        icon: "\u2726",                  // ✦
        description: "OpenAI's coding agent",
        bus: "codex",
    },
    {
        id: "gemini",
        name: "Gemini",
        provider: "gemini",
        icon: "\u26A1",                  // ⚡
        description: "Google's coding agent",
        bus: "gemini",
    },
    {
        id: "kimi",
        name: "Kimi",
        provider: "kimi",
        icon: "\u25C8",                  // ◈
        description: "Moonshot's 262k-context agent",
        bus: "kimi",
    },
    {
        id: "pi",
        name: "Pi",
        provider: "pi",
        icon: "\u03C0",                  // π
        description: "Plandex's multi-provider agent",
        bus: "pi",
    },
    {
        id: "openclaw",
        name: "OpenClaw",
        provider: "openclaw",
        icon: "\u25CE",                  // ◎
        description: "ACP orchestration platform",
        bus: "openclaw",
    },
    // NOTE: GitHub Copilot CLI is documented in the catalog but not
    // registered in frontend PROVIDERS yet — launching it would fail.
    // Re-add once `providers/index.ts` has a copilot entry with
    // launchArgs, controllerType, authCheckCommand, etc.
];

const manifest = {
    version: 5,
    agents: CLI_DEFS.map((d) => ({
        id: d.id,
        name: d.name,
        icon: d.icon,
        provider: d.provider,
        agent_type: "host",
        description: d.description,
        working_directory: "",
        shell: "pwsh",
        agent_bus_id: d.bus,
        content: {
            env: `AGENT_NAME=${d.id}\nAGENTMUX_AGENT_ID=${d.name}`,
            startup: STARTUP,
        },
        skills: [SKILL],
    })),
};

// Write the manifest to `agentmux-srv/forge-seed.json` directly
// rather than emitting it on stdout. Previously this script did
// `process.stdout.write(...)` and callers redirected via
// `node gen-seed.js > forge-seed.json`. On Windows PowerShell,
// stdout redirection transcodes the Node UTF-8 output through the
// console code page (cp437 / cp1252), corrupting the emoji icons
// (🔴 → "≡ƒö┤") and em-dashes (— → "ΓÇö"). Git Bash preserves bytes
// but there was no way to enforce the caller's shell. Writing
// directly sidesteps the issue.
//
// Kept the stdout emit too for callers that pipe elsewhere (e.g.
// diff tools); gated behind `--stdout`.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const json = JSON.stringify(manifest, null, 2) + "\n";

if (process.argv.includes("--stdout")) {
    process.stdout.write(json);
} else {
    const outPath = resolve(__dirname, "..", "agentmux-srv", "forge-seed.json");
    writeFileSync(outPath, json, { encoding: "utf8" });
    process.stdout.write(`wrote ${outPath}\n`);
}
