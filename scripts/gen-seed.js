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
// `crates/srv/src/backend/agent_seed.rs` deletes old seeded rows
// and inserts the new set on version bump — user customisations to
// the old rows are lost. This is intentional: the old layout
// conflated 6 agents into 3 providers, which no longer matches the
// "one card per CLI" model the picker now shows.
//
// Bundles (added 2026-06-18): the manifest also seeds bundles that
// pre-populate Memory → Global and Memory → Bundles.
// Two tiers:
//   is_global: true  — injected into every agent's CLAUDE.md at launch
//   is_global: false — available in the manager but not auto-injected

const SKILL = {
    name: "Startup Verification",
    trigger: "startup",
    skill_type: "prompt",
    description: "Re-run tool verification checks and report status table",
    content:
        "Check your GitHub identity with `gh-agent auth status` where `gh-agent` is installed (plain `gh` reports \"not logged in\" for agents on purpose; never run `gh auth login`, and run `gh-agent doctor` if the check fails; if neither `gh-agent` nor `gh` is installed, as in some containers, report GitHub as N/A), your git identity (`git config user.name` and `git config user.email`), and that the AgentMux MCP server is connected (list peer agents). Report a status table with OK/FAIL for each check. Fix any failures automatically if possible.",
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
        containerImage: "ghcr.io/agentmuxai/agent-base:latest",
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
        id: "antigravity",
        name: "Antigravity",
        provider: "antigravity",
        icon: "\u269B",                  // ⚛
        description: "Google's agentic coding CLI harness (AGY)",
        bus: "antigravity",
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
        description: "Minimal multi-provider coding agent",
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

// ── Seeded bundles ─────────────────────────────────────────────────────
//
// Sourced from a5af/claw workspace patterns (CLAUDE_CONTAINER.md, agent-seed
// startup knowledge). The "Workspace Rules" bundle is global — injected into
// every agent's CLAUDE.md so agents always operate within the same baseline
// git/task/tool rules. The "AgentMux Development" bundle is opt-in — users
// select it when opening an agent to work on the agentmux codebase.

const MEMORY_WORKSPACE_RULES = `## Workspace Rules

### Git Workflow

- **Never push directly to main.** Always create a feature branch first.
- Branch names: \`<agent-name>/feature-description\`.
- Every code change goes through a PR — no direct pushes, no exceptions.
- Never reuse a branch after its PR is merged; create a new branch.
- Resolve merge conflicts rather than force-pushing or discarding changes.

### Task Management

- Use task tracking for any task with 3 or more steps.
- Mark a task **in_progress** before starting it.
- Mark a task **completed** immediately when done — do not batch.

### GitHub Access

| Tier | Tool | Use when |
|------|------|---------|
| 1 | MCP \`mcp__github__*\` tools | Primary — tokens auto-refresh |
| 2 | \`gh\` CLI | MCP unavailable |

### Safety

- Kill processes by PID only — never by image name (kills all instances).
- Never skip pre-commit hooks (\`--no-verify\`) without explicit user approval.
- Confirm before any destructive operation: force-push, reset --hard, branch -D.`;

const MEMORY_AGENTMUX_DEV = `## AgentMux Development

Context for working on the AgentMux codebase. Add this bundle to an agent
that works on agentmux.

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

### muxreview Bot

All PRs are auto-reviewed by muxreview (Claude Opus). Address P1 findings before
merging. P2 findings should also be fixed if feasible.

### Critical Rule

**NEVER kill AgentMux by image name** — use PID only. Multiple instances share
the same binary name; killing by name kills ALL of them.`;

const MEMORY_AGENT_MEMORY = "## Agent Memory\n\nYour persistent memory lives in `~/.claude/projects/<path>/memory/`. Claude Code reads it automatically at session start.\n\n### Structure\n\n- **`MEMORY.md`** - Index file. Loaded into *every* session (200-line / ~25 KB limit). Keep it as a pointer list. Entries past 200 lines are truncated.\n- **Topic files** (`*.md`) - Detail files (e.g. `user_role.md`, `feedback_testing.md`). Loaded on-demand when relevant. Store depth here, not in MEMORY.md.\n\n### File format\n\nEach topic file:\n```markdown\n---\nname: short-kebab-slug\ndescription: one-line summary\nmetadata:\n  type: user | feedback | project | reference\n---\n\nBody content here.\n```\n\nMEMORY.md - one line per entry:\n```markdown\n- [Title](file.md) - one-line hook\n```\n\n### When to save\n\nSave when you learn something non-obvious that would help future sessions:\n- User preferences, role, expertise (type: user)\n- Corrections or confirmed approaches: avoid X, always do Y (type: feedback)\n- Project context not in code/git: deadlines, stakeholder decisions, why (type: project)\n- Where things live in external systems: Slack, dashboards, Linear (type: reference)\n\nDo NOT save:\n- Things derivable from code or git history\n- Ephemeral task state or in-progress work\n- Duplicates - update existing entries instead\n- Anything already in CLAUDE.md files\n\n### When to update\n\n- End of a session where you learned something worth keeping\n- When the user corrects you\n- When you discover project context the user has not written down\n- When you find where something lives in an external system\n\n### Discipline\n\n- MEMORY.md stays under 200 lines. Consolidate if it grows past ~150.\n- One file per topic - do not dump everything in MEMORY.md body.\n- Link topic files from MEMORY.md with [[name]] cross-references.\n- Stale memories rot - update or remove when things change.";

const SEED_MEMORIES = [
    {
        id: "seed-agent-memory",
        name: "Agent Memory",
        description: "When and how agents read and update their persistent native memory",
        is_global: true,
        instructions: MEMORY_AGENT_MEMORY,
    },
    {
        id: "seed-workspace-rules",
        name: "Workspace Rules",
        description: "Git workflow, task management, GitHub access tiers, and safety rules — injected into all agents",
        is_global: true,
        instructions: MEMORY_WORKSPACE_RULES,
    },
    {
        id: "seed-agentmux-dev",
        // Earlier texts of this bundle (SHA-256): an install still holding one
        // exactly gets the new text; an edited bundle is left alone.
        replaces_instructions_sha256: ["04af0430edeaa5c0acbdae583e239a880061221c9406be5a4eeff318c8a2f34e"],
        name: "AgentMux Development",
        description: "Stack, build commands, version management, and workflow for the AgentMux codebase",
        is_global: false,
        instructions: MEMORY_AGENTMUX_DEV,
    },
];

const manifest = {
    version: 11,
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
            // "__SKIP__" suppresses the startup injection. The /startup skill
            // is the opt-in path for users who want the verification round.
            startup: "__SKIP__",
        },
        skills: [SKILL],
    })),
    memories: SEED_MEMORIES,
};

// Write the manifest to `crates/srv/agent-seed.json` directly
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
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const json = JSON.stringify(manifest, null, 2) + "\n";

const outPath = resolve(__dirname, "..", "crates", "srv", "agent-seed.json");
if (process.argv.includes("--check")) {
    const committed = JSON.parse(readFileSync(outPath, "utf8"));
    if (JSON.stringify(committed) !== JSON.stringify(manifest)) {
        process.stderr.write("gen-seed: crates/srv/agent-seed.json differs from scripts/gen-seed.js; edit the script and run it.\n");
        process.exit(1);
    }
    process.stdout.write("gen-seed: agent-seed.json matches.\n");
} else if (process.argv.includes("--stdout")) {
    process.stdout.write(json);
} else {
    writeFileSync(outPath, json, { encoding: "utf8" });
    process.stdout.write(`wrote ${outPath}\n`);
}
