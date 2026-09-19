#!/usr/bin/env bash
# gh-agent.sh — run `gh` authenticated as the calling agent's own GitHub
# identity, never the machine-wide `gh auth login` session.
#
# Problem: on a machine running multiple agents, `gh` with no token override
# falls back to whichever account last ran `gh auth login` in the shared
# keyring config — e.g. Agent2's shell inheriting Agent-Y's login. That's
# silently wrong: PRs/comments get attributed to the wrong account.
#
# Fix, three-tier as of SPEC_GITHUB_APP_IDENTITY_MIGRATION_2026_09_18.md:
#   1. GitHub App installation token, minted fresh on every call from this
#      agent's own github_app_id/github_app_installation_id/<agent>-workflow-key
#      (services/infra:agent-configs.<agent>) via scripts/github-app-token.py.
#      Nothing to rotate — it expires on its own in an hour — and nothing that
#      can go stale in a git remote URL the way a long-lived PAT can.
#   2. Falls back to the shared genericagentx-workflow App identity (same
#      github-app-token.py path, agent name "genericagentx") for any agent
#      that doesn't have its own dedicated App provisioned yet — still a
#      short-lived token, not a static PAT, so this tier doesn't reintroduce
#      the long-lived-credential problem the whole migration exists to fix.
#   3. Falls back to the long-lived gh-token-<agent> PAT (or the shared
#      gh-token-genericagentx account) only if BOTH App paths fail for any
#      reason — this script must keep working regardless of migration
#      progress, so a fallback failure never turns into a hard failure here.
# Either way, the token is passed as GH_TOKEN scoped to just this one `gh`
# invocation — never written to disk, never touches the shared keyring,
# resolved fresh every call so it tracks whichever agent is running.
#
# Usage: scripts/gh-agent.sh <gh subcommand and args...>
#   e.g. scripts/gh-agent.sh pr create --title "..." --body "..."
#        scripts/gh-agent.sh auth status
#
# Requires: $AGENTMUX_AGENT_ID set (injected at agent spawn), `secrets`
# (@a5af/secrets CLI) on PATH, and `python3` + PyJWT + requests for the App
# token path specifically (missing/failing any of these just falls through
# to the PAT path, per the design above).

set -euo pipefail

if [[ -z "${AGENTMUX_AGENT_ID:-}" ]]; then
    echo "gh-agent: \$AGENTMUX_AGENT_ID is not set — refusing to guess an identity." >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SECRET_ID="services/infra"
AGENT_LOWER="$(printf '%s' "$AGENTMUX_AGENT_ID" | tr '[:upper:]' '[:lower:]')"
FALLBACK_KEY="gh-token-genericagentx"

# An installation token is scoped to exactly one account/org - resolve which
# one owns the repo we're actually operating on from the local git remote,
# so agents whose App is installed on multiple orgs (e.g. a5af AND
# agentmuxai) mint a token that actually has access to the target repo.
# Defaults to a5af (the historical single-org case) if there's no git
# remote to inspect (e.g. `gh auth status` run outside a repo).
TARGET_ORG="a5af"
if REMOTE_URL="$(git remote get-url origin 2>/dev/null)"; then
    if [[ "$REMOTE_URL" =~ [:/]([^/:]+)/[^/]+\.git$ || "$REMOTE_URL" =~ [:/]([^/:]+)/[^/]+$ ]]; then
        TARGET_ORG="${BASH_REMATCH[1]}"
    fi
fi

TOKEN=""
USED_KEY=""
AUTH_MODE=""

# --- Tier 1: this agent's own GitHub App installation token ----------------
if command -v python3 >/dev/null 2>&1; then
    APP_ERR_FILE="$(mktemp)"
    if APP_TOKEN="$(python3 "$SCRIPT_DIR/github-app-token.py" "$AGENT_LOWER" "$TARGET_ORG" 2>"$APP_ERR_FILE")"; then
        TOKEN="$APP_TOKEN"
        USED_KEY="${AGENT_LOWER}-workflow-key"
        AUTH_MODE="app"
    elif [[ "$(cat "$APP_ERR_FILE" 2>/dev/null)" != *"no App identity"* ]]; then
        # Exit code wasn't the clean "no identity provisioned" case (2) -
        # something about minting actually failed. Surface it (stderr only,
        # never fatal) since a silently-failing App path that always falls
        # back to the next tier would hide the exact class of bug this
        # migration exists to prevent from recurring unnoticed.
        echo "gh-agent: own App token mint failed, trying shared App:" >&2
        cat "$APP_ERR_FILE" >&2
    fi
    rm -f "$APP_ERR_FILE"
fi

# --- Tier 2: shared genericagentx-workflow App installation token ----------
if [[ -z "$TOKEN" ]] && command -v python3 >/dev/null 2>&1; then
    SHARED_ERR_FILE="$(mktemp)"
    if SHARED_TOKEN="$(python3 "$SCRIPT_DIR/github-app-token.py" genericagentx "$TARGET_ORG" 2>"$SHARED_ERR_FILE")"; then
        TOKEN="$SHARED_TOKEN"
        USED_KEY="genericagentx-workflow-key"
        AUTH_MODE="app"
    elif [[ "$(cat "$SHARED_ERR_FILE" 2>/dev/null)" != *"no App identity"* ]]; then
        echo "gh-agent: shared App token mint failed, falling back to PAT:" >&2
        cat "$SHARED_ERR_FILE" >&2
    fi
    rm -f "$SHARED_ERR_FILE"
fi

# --- Tier 3: long-lived PAT fallback ----------------------------------------
if [[ -z "$TOKEN" ]]; then
    OWN_KEY="gh-token-${AGENT_LOWER}"
    TOKEN="$(secrets get "$SECRET_ID" --path "$OWN_KEY" --raw --no-warning 2>/dev/null || true)"
    USED_KEY="$OWN_KEY"
    if [[ -z "$TOKEN" ]]; then
        TOKEN="$(secrets get "$SECRET_ID" --path "$FALLBACK_KEY" --raw --no-warning)"
        USED_KEY="$FALLBACK_KEY"
    fi
    AUTH_MODE="pat"
fi

if [[ -z "$TOKEN" ]]; then
    echo "gh-agent: could not resolve an App token or a PAT for '$AGENTMUX_AGENT_ID'." >&2
    exit 1
fi

echo "gh-agent: authenticating as $AGENTMUX_AGENT_ID via $AUTH_MODE:$USED_KEY" >&2

# On the SHARED identity (PAT or App), a `pr create` without the
# machine-readable body tag gets its review notifications silently dropped.
# Per SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md the consumer resolves the
# agent from the PR author's GitHub username FIRST and falls back to the tag
# only when that username is this shared account -- so on a dedicated PAT or
# App identity (each its own distinct bot username) the tag is redundant,
# and here it is the ONLY thing saying which agent opened the PR. The
# genericagentx-workflow App is just as shared across agents as the PAT was,
# so it needs the same warning.
#
# This warning exists because the failure is invisible: the PR opens fine, CI
# runs fine, the review posts fine, and the only symptom is jekts that never
# arrive -- which reads as "quiet", not "broken". It cost one agent eleven PRs
# of manual polling before anyone noticed.
if [[ "$USED_KEY" == "$FALLBACK_KEY" || "$USED_KEY" == "genericagentx-workflow-key" ]]; then
    for _arg in "$@"; do
        if [[ "$_arg" == "create" ]]; then
            {
                echo "gh-agent: WARNING -- shared identity. A 'pr create' MUST carry"
                echo "          <!-- agentmux:agent_id=<your-id-lowercased> -->   in the BODY"
                echo "          (an HTML comment; plain text like 'agent_id: x' is NOT parsed)"
                echo "          plus a '<Agent>@<host>: ' TITLE prefix."
                echo "          Without the tag, your review notifications are dropped silently."
            } >&2
            break
        fi
    done
fi

GH_TOKEN="$TOKEN" exec gh "$@"
