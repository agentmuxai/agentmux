#!/usr/bin/env bash
# gh-agent.sh — run `gh` authenticated as the calling agent's own GitHub
# identity, never the machine-wide `gh auth login` session.
#
# Problem: on a machine running multiple agents, `gh` with no token override
# falls back to whichever account last ran `gh auth login` in the shared
# keyring config — e.g. Agent2's shell inheriting Agent-Y's login. That's
# silently wrong: PRs/comments get attributed to the wrong account.
#
# Fix: resolve this agent's own PAT from AWS Secrets Manager (via
# @a5af/secrets, see a5af/dev-tools) by convention `gh-token-<agent, lower>`,
# falling back to the shared `gh-token-genericagentx` account for agents that
# don't have a dedicated PAT registered. Pass it as GH_TOKEN scoped to just
# this one `gh` invocation — never written to disk, never touches the shared
# keyring, resolved fresh every call so it tracks whichever agent is running.
#
# Usage: scripts/gh-agent.sh <gh subcommand and args...>
#   e.g. scripts/gh-agent.sh pr create --title "..." --body "..."
#        scripts/gh-agent.sh auth status
#
# Requires: $AGENTMUX_AGENT_ID set (injected at agent spawn) and `secrets`
# (@a5af/secrets CLI) on PATH.

set -euo pipefail

if [[ -z "${AGENTMUX_AGENT_ID:-}" ]]; then
    echo "gh-agent: \$AGENTMUX_AGENT_ID is not set — refusing to guess an identity." >&2
    exit 1
fi

SECRET_ID="services/infra"
AGENT_LOWER="$(printf '%s' "$AGENTMUX_AGENT_ID" | tr '[:upper:]' '[:lower:]')"
OWN_KEY="gh-token-${AGENT_LOWER}"
FALLBACK_KEY="gh-token-genericagentx"

TOKEN="$(secrets get "$SECRET_ID" --path "$OWN_KEY" --raw --no-warning 2>/dev/null || true)"
USED_KEY="$OWN_KEY"
if [[ -z "$TOKEN" ]]; then
    TOKEN="$(secrets get "$SECRET_ID" --path "$FALLBACK_KEY" --raw --no-warning)"
    USED_KEY="$FALLBACK_KEY"
fi

if [[ -z "$TOKEN" ]]; then
    echo "gh-agent: could not resolve a PAT from '$OWN_KEY' or '$FALLBACK_KEY'." >&2
    exit 1
fi

echo "gh-agent: authenticating as $AGENTMUX_AGENT_ID via secrets:$USED_KEY" >&2
GH_TOKEN="$TOKEN" exec gh "$@"
