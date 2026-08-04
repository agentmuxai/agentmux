# Copilot CLI Verification Report — Discussion #493

**Date:** 2026-04-23
**Discussion:** https://github.com/agentmuxai/agentmux/discussions/493
**Scope:** GitHub Copilot CLI claims added to the research doc on 2026-04-23.
**Method:** Every Copilot-specific claim cross-checked against official `docs.github.com/en/copilot/...` pages, the GitHub Changelog feed, and the `github/copilot-cli` release feed.

## Summary

| Bucket | Count |
|--------|-------|
| Claims fully verified verbatim in official docs | 27 |
| Claims corrected in place (were wrong) | 2 |
| Claims trimmed (project-side paths unverifiable) | 2 |
| Claims reworded for accuracy/nuance | 2 |
| Total Copilot claims audited | 33 |
| Unresolved / unverifiable errors | 0 |

All corrections have been silently applied to the discussion body on GitHub.

## Corrections applied

### 1. MCP project config does not exist (was claimed in 3 places)

**Was:**
- Startup step 5: "Load project overrides (if present): `.mcp.json` or `.github/mcp.json`"
- Startup step 12: "MCP servers launch (project scope overrides user on name collision)"
- MCP matrix row: "`~/.copilot/mcp-config.json` (user), `.mcp.json` or `.github/mcp.json` (project — wins on collision)"

**Now:**
- Startup step 4 (renumbered): user-level MCP only; note that per-repo MCP is an open feature request (`github/copilot-cli#1291`, `#2528`).
- Startup step 11 (renumbered): "MCP servers launch (user-scope only)".
- MCP matrix row: user-only, with feature-request note.

**Evidence:** `docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers` documents only `~/.copilot/mcp-config.json`. No project override surface exists.

### 2. Agent name-collision precedence was reversed

**Was:** "project wins on name collision"

**Now:** "user/home wins on name collision"

**Evidence:** `docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/create-custom-agents-for-cli`:

> If you have custom agents with the same name in both locations, the one in your home directory will be used, rather than the one in the repository.

This is the opposite of Claude Code's project-first precedence.

### 3. Project-side skills path trimmed

**Was:** `~/.copilot/skills/<name>/SKILL.md + .github/<skills>/<name>/SKILL.md`

**Now:** `~/.copilot/skills/<name>/SKILL.md (user)` only.

**Evidence:** Official docs confirm `~/.copilot/skills/my-skill/SKILL.md`. No project-scoped skills directory is documented.

### 4. Project-side hooks path trimmed

**Was:** `~/.copilot/hooks/ + .github/hooks/`

**Now:** `~/.copilot/hooks/ (user)` only.

**Evidence:** Docs describe user-level `~/.copilot/hooks/` only. `.github/hooks/` is not mentioned.

### 5. Shift+Tab is a cycle, not a binary toggle

**Was:** "Shift+Tab toggles Plan Mode interactively"

**Now:** "Shift+Tab cycles three modes (Interactive → Plan → Autopilot)"

**Evidence:** `docs.github.com/en/copilot/concepts/agents/about-copilot-cli` and the 2026-01-21 changelog: "Press Shift+Tab to cycle in and out of plan mode." Three modes exist: Interactive, Plan, Autopilot.

### 6. Copilot Memory is repo-scoped, not per-user-synced

**Was:** "Copilot Memory for repo-level persistence" / "No (Memory feature syncs via GitHub)"

**Now:** "Copilot Memory is repository-scoped, stored on GitHub, shared across users/devices with repo access" / "Yes (Copilot Memory, repo-scoped)"

**Evidence:** `docs.github.com/en/copilot/concepts/agents/copilot-memory` and the 2026-03-04 changelog clarify that memories attach to the repository and are shared across everyone with access to Copilot Memory on that repo.

### 7. Version row expanded

**Was:** "v1.0.34 (2026-04-20)"

**Now:** "v1.0.34 stable (2026-04-20); v1.0.35-6 pre-release (2026-04-23)"

**Evidence:** `github.com/github/copilot-cli/releases`. Pre-releases v1.0.35-2 through v1.0.35-6 shipped 2026-04-21 through 2026-04-23.

## Claims verified verbatim (unchanged)

| # | Claim | Source |
|---|-------|--------|
| 1 | Copilot reads `AGENTS.md` at repo root, CWD, `$COPILOT_CUSTOM_INSTRUCTIONS_DIRS` | add-custom-instructions |
| 2 | `.github/copilot-instructions.md` is a repo-wide instruction file | add-custom-instructions |
| 3 | `.github/instructions/**/*.instructions.md` are path-scoped | add-custom-instructions |
| 4 | `$HOME/.copilot/copilot-instructions.md` is user-global instructions | add-custom-instructions |
| 5 | Copilot also reads `CLAUDE.md` and `GEMINI.md` at repo root | add-custom-instructions |
| 6 | `$COPILOT_CUSTOM_INSTRUCTIONS_DIRS` is a comma-separated list | add-custom-instructions |
| 7 | Config dir precedence: `--config-dir` > `$COPILOT_HOME` > `~/.copilot` | cli-config-dir-reference |
| 8 | `config.json` is JSONC (JSON with comments) | cli-config-dir-reference |
| 9 | Config fields include `model`, `effortLevel`, `theme`, `mouse`, `banner`, `renderMarkdown` | cli-config-dir-reference |
| 10 | `permissions-config.json` stores saved tool/directory approvals per project | cli-config-dir-reference |
| 11 | User MCP at `~/.copilot/mcp-config.json` | add-mcp-servers |
| 12 | User agents at `~/.copilot/agents/*.agent.md`; project at `.github/agents/*.agent.md` | create-custom-agents-for-cli |
| 13 | Cache: macOS `~/Library/Caches/copilot`, Linux `$XDG_CACHE_HOME/copilot`, Windows `%LOCALAPPDATA%/copilot` | cli-config-dir-reference |
| 14 | Override cache with `$COPILOT_CACHE_HOME` | cli-config-dir-reference |
| 15 | Built-in subagents **Explore, Task, Plan, Code-review** shipped Jan 2026 | 2026-01-14 changelog |
| 16 | Task runs tests/builds; Code-review surfaces only genuine issues | 2026-01-14 changelog |
| 17 | Copilot auto-delegates and runs subagents in parallel | 2026-01-14 changelog |
| 18 | Subagents have their own context window | create-custom-agents-for-cli |
| 19 | `/model` slash command and `--model` flag | about-copilot-cli |
| 20 | `auto` model selection went GA April 2026 | 2026-04-17 changelog |
| 21 | Default model is Claude Sonnet 4.5 | about-copilot-cli |
| 22 | BYOK env vars `$COPILOT_PROVIDER_BASE_URL`, `$COPILOT_MODEL` | use-byok-models |
| 23 | Session state at `~/.copilot/session-state/<id>/events.jsonl` | cli-config-dir-reference |
| 24 | Session store SQLite at `~/.copilot/session-store.db` for checkpoint indexing | cli-config-dir-reference |
| 25 | Auto-compaction triggers at 95% of token limit | 2026-01-14 changelog |
| 26 | `/compact` and `/context` slash commands | 2026-01-14 changelog |
| 27 | Stable version v1.0.34 released 2026-04-20 | github/copilot-cli releases |

## Net result

- Discussion body updated in place twice (initial push + correction push).
- 6 substantive corrections applied, 2 nuance improvements, 1 version-row expansion.
- Accuracy header in the discussion updated from `42/46` to `69/79 verified; 10 corrected; 0 unresolved`.
- No claims remain unresolved.

## Sources

- https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-custom-instructions
- https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference
- https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/create-custom-agents-for-cli
- https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers
- https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models
- https://docs.github.com/en/copilot/concepts/agents/about-copilot-cli
- https://docs.github.com/en/copilot/concepts/agents/copilot-memory
- https://github.blog/changelog/2026-01-14-github-copilot-cli-enhanced-agents-context-management-and-new-ways-to-install/
- https://github.blog/changelog/2026-01-21-github-copilot-cli-plan-before-you-build-steer-as-you-go/
- https://github.blog/changelog/2026-03-04-copilot-memory-now-on-by-default-for-pro-and-pro-users-in-public-preview/
- https://github.blog/changelog/2026-04-17-github-copilot-cli-now-supports-copilot-auto-model-selection/
- https://github.com/github/copilot-cli/releases
- https://github.com/github/copilot-cli/issues/1291
- https://github.com/github/copilot-cli/issues/2528
