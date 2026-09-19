#!/usr/bin/env bash
# git-credential-agentmux.sh — a git credential helper that mints a
# short-lived GitHub App installation token for the calling agent.
#
# WHY THIS EXISTS
#
# `git push` on an agent host currently authenticates one of two ways, and
# both are bad:
#
#   1. The machine-wide helper `gh auth git-credential`, which reads
#      ~/.config/gh/hosts.yml. On this machine that file holds the shared
#      a5af PAT: 21 scopes, no expiry, including admin:org and delete_repo.
#      So an ordinary `git push` runs as the account owner with rights to
#      delete repositories.
#   2. A long-lived per-agent PAT embedded in the remote URL
#      (https://x-access-token:ghp_...@github.com/...). Better, but the
#      token is long-lived, lands in shell history and process listings,
#      and has been observed copied between agents' .git-credentials files.
#
# This helper replaces both. Git invokes it, it mints an installation token
# scoped to the repo's owner, and the token expires in an hour. Nothing is
# written to disk and nothing long-lived exists to leak.
#
# An App installation token works in the x-access-token position exactly as
# a PAT does -- already proven in production by nightly-release.yml.
#
# SETUP (per agent, or machine-wide)
#
#   git config --global credential.https://github.com.helper \
#       '!bash /path/to/scripts/git-credential-agentmux.sh'
#   git config --global credential.https://github.com.useHttpPath true
#
# useHttpPath is REQUIRED, not optional: without it git does not tell the
# helper which repository is being accessed, and installation tokens are
# per-account -- we would not know which installation to mint against.
#
# FALLBACK: if minting fails for any reason (no App for this agent, no AWS
# credentials, no python3), the helper prints nothing and exits 0. Git then
# moves on to the next configured helper, so pushes keep working rather than
# hard-failing. Set AGENTMUX_GIT_CRED_STRICT=1 to fail loudly instead.

set -uo pipefail

# Git calls helpers as: <helper> get|store|erase
[[ "${1:-}" == "get" ]] || exit 0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Read git's key=value request from stdin.
host=""; path=""; protocol=""
while IFS= read -r line; do
    [[ -z "$line" ]] && break
    case "$line" in
        host=*)     host="${line#host=}" ;;
        path=*)     path="${line#path=}" ;;
        protocol=*) protocol="${line#protocol=}" ;;
    esac
done

[[ "$host" == "github.com" ]] || exit 0

# path looks like "owner/repo.git" (or "owner/repo"). We need the owner, since
# installation tokens are scoped to one account.
owner="${path%%/*}"
if [[ -z "$owner" ]]; then
    # No useHttpPath, so we cannot know the installation. Say nothing and let
    # the next helper try, rather than guessing an owner and minting a token
    # for the wrong account.
    [[ "${AGENTMUX_GIT_CRED_STRICT:-0}" == "1" ]] && {
        echo "git-credential-agentmux: no path in request; set credential.https://github.com.useHttpPath=true" >&2
        exit 1
    }
    exit 0
fi

agent="$(printf '%s' "${AGENTMUX_AGENT_ID:-}" | tr '[:upper:]' '[:lower:]')"
[[ -n "$agent" ]] || agent="genericagentx"

token=""
if command -v python3 >/dev/null 2>&1; then
    # Tier 1: this agent's own App. Tier 2: the shared App. Same order as
    # gh-agent.sh, for the same reason -- an agent without its own App must
    # still get a short-lived token rather than falling back to a PAT.
    token="$(python3 "$SCRIPT_DIR/github-app-token.py" "$agent" "$owner" 2>/dev/null)" || token=""
    # Only fall back when tier 1 used a DIFFERENT identity. If
    # AGENTMUX_AGENT_ID is unset, $agent already defaulted to genericagentx
    # above, so retrying it repeats an identical call that can only fail the
    # same way -- doubling latency and API calls on every push made outside
    # an agent context.
    if [[ -z "$token" && "$agent" != "genericagentx" ]]; then
        token="$(python3 "$SCRIPT_DIR/github-app-token.py" genericagentx "$owner" 2>/dev/null)" || token=""
    fi
fi

if [[ -z "$token" ]]; then
    if [[ "${AGENTMUX_GIT_CRED_STRICT:-0}" == "1" ]]; then
        echo "git-credential-agentmux: could not mint an App token for '$owner' as '$agent'" >&2
        exit 1
    fi
    exit 0   # stay silent; git falls through to the next helper
fi

printf 'protocol=%s\n' "${protocol:-https}"
printf 'host=%s\n' "$host"
printf 'username=x-access-token\n'
printf 'password=%s\n' "$token"
