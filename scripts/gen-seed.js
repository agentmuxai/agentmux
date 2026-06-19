// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Generates agent-seed.json — one seeded definition per CLI in the
// catalog (frontend/app/view/agent/defaults/cli-catalog.ts). Each row
// is a template; a running instance is created when the user picks a
// card + supplies a name via AgentLaunchModal.
//
// See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md §5.
//
// Manifest version 5 (2026-04-23): replaces the AgentX/Y/Z + Agent1/2/3
// layout with per-CLI definitions. The re-seed engine in
// `agentmux-srv/src/backend/agent_seed.rs` deletes old seeded rows
// and inserts the new set on version bump — user customisations to
// the old rows are lost. This is intentional: the old layout
// conflated 6 agents into 3 providers, which no longer matches the
// "one card per CLI" model the picker now shows.
//
// Memory bundles (added 2026-06-18): the manifest also seeds Memory bundles
// that pre-populate the Trust Center (Identity & Memory hamburger modal).
// Two tiers:
//   is_global: true  — injected into every agent's CLAUDE.md at launch
//   is_global: false — available in the manager but not auto-injected

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
        containerSupported: true,
        containerImage: "ghcr.io/agentmuxai/agent-claude:latest",
    },
    {
        id: "codex",
        name: "Codex",
        provider: "codex",
        icon: "\u2726",                  // ✦
        description: "OpenAI's coding agent",
        bus: "codex",
        // No container image built yet — host-only until agentmux/codex ships.
        containerSupported: false,
    },
    {
        id: "gemini",
        name: "Gemini",
        provider: "gemini",
        icon: "\u26A1",                  // ⚡
        description: "Google's coding agent",
        bus: "gemini",
        // No container image built yet — host-only until agentmux/gemini ships.
        containerSupported: false,
    },
    {
        id: "kimi",
        name: "Kimi",
        provider: "kimi",
        icon: "\u25C8",                  // ◈
        description: "Moonshot's 262k-context agent",
        bus: "kimi",
        // No container image built yet — host-only until agentmux/kimi ships.
        containerSupported: false,
    },
    {
        id: "pi",
        name: "Pi",
        provider: "pi",
        icon: "\u03C0",                  // π
        description: "Plandex's multi-provider agent",
        bus: "pi",
        // No container image built yet — host-only until agentmux/pi ships.
        containerSupported: false,
    },
    {
        id: "openclaw",
        name: "OpenClaw",
        provider: "openclaw",
        icon: "\u25CE",                  // ◎
        description: "ACP orchestration platform",
        bus: "openclaw",
        // Needs host access to orchestrate other agents.
        containerSupported: false,
    },
    {
        id: "copilot",
        name: "Copilot",
        provider: "copilot",
        icon: "\u26F6",                  // ⛶
        description: "Microsoft's coding agent",
        bus: "copilot",
        // No container image built yet — host-only until agentmux/copilot ships.
        containerSupported: false,
    },
];

// ── Seeded Memory bundles ─────────────────────────────────────────────────────
//
// Sourced from a5af/claw workspace patterns (CLAUDE_CONTAINER.md, agent-seed
// startup knowledge). The "Workspace Rules" bundle is global — injected into
// every agent's CLAUDE.md so agents always operate within the same baseline
// git/task/tool rules. The "AgentMux Development" bundle is opt-in — users
// select it when opening an agent to work on the agentmux codebase.

const MEMORY_WORKSPACE_RULES = `## Workspace Rules

These rules govern how you operate in this multi-agent workspace. They apply
regardless of which project you are working on.

### Git Workflow

- **Never push directly to main.** Always create a feature branch first.
- Branch names follow the pattern \`<agent-name>/feature-description\`.
- Always create a PR for code changes — no direct pushes.
- Never reuse a branch after its PR is merged; create a new branch.
- Resolve merge conflicts rather than force-pushing or discarding changes.

### Task Management

- Use task tracking for any task with 3 or more steps.
- Mark a task **in_progress** before you start it.
- Mark a task **completed** immediately when it is done — do not batch.

### GitHub Access

| Tier | Tool | Use when |
|------|------|---------|
| 1 | MCP \`mcp__github__*\` tools | Primary — tokens auto-refresh |
| 2 | \`gh\` CLI | MCP unavailable |
| 3 | Admin PAT via \`secrets\` | Package publish, admin ops only |

### Dev Tools

Verify tools are available: \`secrets --version && deploy --version && reagent --version\`

Install if missing:
\`\`\`bash
npm install -g @a5af/secrets @a5af/deploy-cli @a5af/reagent-cli @a5af/workspace-health
\`\`\`

### Safety

- Kill processes by PID only — never by image name (kills all instances).
- Never skip pre-commit hooks (\`--no-verify\`) without explicit user approval.
- Confirm before destructive operations: force-push, reset --hard, branch -D.`;

const MEMORY_AGENTMUX_DEV = `## AgentMux Development

Context for working on the AgentMux codebase. Select this Memory bundle when
opening an agent to work on agentmux.

### Stack

- **agentmux-cef** — Chromium host, window management, IPC bridge
- **agentmux-launcher** — Job Object, single-instance pipe, saga coordinator
- **agentmux-srv** — Async Rust backend (SQLite, agent/block management)
- **agentmux-common** — Shared types and utilities
- **Frontend** — SolidJS + SCSS, built with Vite

### Build Commands

| Command | Purpose |
|---------|---------|
| \`task dev\` | Development (hot reload) |
| \`task package\` | Portable ZIP build |
| \`task build:backend\` | Rebuild Rust sidecar after changes |
| \`task test\` | Run tests |

After Rust changes: \`task build:backend\`, then restart \`task dev\`.

### Version Management

Feature PRs use changesets — do NOT bump versions manually:
\`\`\`bash
task changeset -- patch "fix(scope): short description"
\`\`\`

### Logs

\`muxlog host\` / \`muxlog srv\` / \`muxlog fe\` — pipe to \`grep\` for filtering.

### Reagent Bot

All PRs are auto-reviewed by reagent (Claude Opus). Address P1 findings before
merging. P2 findings should also be fixed if feasible.

### Critical Rule

**NEVER kill AgentMux by image name** — use PID only. Multiple instances share
the same binary name; killing by name kills ALL of them.`;

const SEED_MEMORIES = [
    {
        id: "seed-workspace-rules",
        name: "Workspace Rules",
        description: "Git workflow, task management, GitHub access tiers, and dev tool rules — injected into all agents",
        is_global: true,
        instructions: MEMORY_WORKSPACE_RULES,
    },
    {
        id: "seed-agentmux-dev",
        name: "AgentMux Development",
        description: "Stack, build commands, version management, and workflow for the AgentMux codebase",
        is_global: false,
        instructions: MEMORY_AGENTMUX_DEV,
    },
];

const manifest = {
    version: 10,
    agents: CLI_DEFS.map((d) => ({
        id: d.id,
        name: d.name,
        icon: d.icon,
        provider: d.provider,
        agent_type: d.containerSupported ? "container" : "host",
        ...(d.containerImage ? { container_image: d.containerImage } : {}),
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
    memories: SEED_MEMORIES,
};

// Write the manifest to `agentmux-srv/agent-seed.json` directly
// rather than emitting it on stdout. Previously this script did
// `process.stdout.write(...)` and callers redirected via
// `node gen-seed.js > agent-seed.json`. On Windows PowerShell,
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
    const outPath = resolve(__dirname, "..", "agentmux-srv", "agent-seed.json");
    writeFileSync(outPath, json, { encoding: "utf8" });
    process.stdout.write(`wrote ${outPath}\n`);
}
