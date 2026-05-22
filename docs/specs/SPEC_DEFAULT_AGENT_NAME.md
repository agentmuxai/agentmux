<!-- Captured from GitHub issue #780 — agentmuxai/agentmux -->

## Summary

The agent-name field is empty on launch form open. User must type a name to click Launch. Friction with no payoff — most users want to just spin up an agent.

## Proposed

Pre-populate with `<Provider> Agent` (e.g., `Claude Agent`). If that name's taken, suffix `2`, `3`, ... — `Claude Agent`, `Claude Agent 2`, `Claude Agent 3`. User can still edit; default just removes the type-something-to-proceed friction.

## Spec

`docs/specs/SPEC_DEFAULT_AGENT_NAME.md` on branch `agenta/spec-default-agent-name`.

Covers: algorithm, where to compute, dirty-flag handling on provider change, validation, effort (~55 LOC, ~0.5 day).

## Related

- #779 — zombie HWND eats keystrokes; this spec is a partial mitigation (no typing needed for the happy path).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
