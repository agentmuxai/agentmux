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

# github-app-token.py has an exit-code contract, and it matters here:
#   0 = token on stdout
#   2 = this agent has no App identity  -> expected, stay quiet
#   1 = the lookup genuinely FAILED (expired AWS credentials, Secrets Manager
#       unreachable, malformed key) -> must be surfaced
#
# Flattening 1 and 2 into "no token" would be actively dangerous. Falling
# through silently hands the request to the NEXT credential helper, which on
# this machine is `gh auth git-credential` reading the 21-scope admin PAT --
# the exact credential this helper exists to stop using. A transient AWS
# outage would then quietly re-escalate every push to admin, with no signal.
# So a real failure always prints why, even in non-strict mode.
token=""
MINT_HARD_FAIL=0
MINT_TOKEN=""

# NOTE: called as `mint ...`, never as `$(mint ...)`. Command substitution runs
# the function in a SUBSHELL, so MINT_HARD_FAIL set inside would be discarded --
# which silently defeated the whole point of this block on the first attempt.
mint() {                      # $1=identity $2=owner ; sets MINT_TOKEN, returns rc
    local err rc
    err="$(mktemp)"
    MINT_TOKEN="$(python3 "$SCRIPT_DIR/github-app-token.py" "$1" "$2" 2>"$err")"
    rc=$?
    if [[ $rc -ne 0 && $rc -ne 2 ]]; then
        echo "git-credential-agentmux: minting failed for '$1' on '$2' (exit $rc) -- NOT falling back silently:" >&2
        sed 's/^/    /' "$err" >&2
        MINT_HARD_FAIL=1
    fi
    [[ $rc -ne 0 ]] && MINT_TOKEN=""
    rm -f "$err"
    return $rc
}

if command -v python3 >/dev/null 2>&1; then
    # Tier 1: this agent's own App. Tier 2: the shared App -- only when tier 1
    # used a DIFFERENT identity, and only if tier 1 did not hard-fail. If
    # AGENTMUX_AGENT_ID is unset, $agent already defaulted to genericagentx, so
    # retrying it repeats an identical call that can only fail the same way.
    mint "$agent" "$owner" || true
    token="$MINT_TOKEN"
    if [[ -z "$token" && $MINT_HARD_FAIL -eq 0 && "$agent" != "genericagentx" ]]; then
        mint genericagentx "$owner" || true
        token="$MINT_TOKEN"
    fi
else
    echo "git-credential-agentmux: python3 not found; cannot mint an App token" >&2
    MINT_HARD_FAIL=1
fi

if [[ -z "$token" ]]; then
    # Strict mode, or a genuine (non-exit-2) failure that we already explained
    # above: refuse rather than let git silently escalate to another helper.
    if [[ "${AGENTMUX_GIT_CRED_STRICT:-0}" == "1" ]]; then
        echo "git-credential-agentmux: no App token for '$owner' as '$agent' (strict mode)" >&2
        exit 1
    fi
    if [[ $MINT_HARD_FAIL -eq 1 ]]; then
        echo "git-credential-agentmux: falling through to the next helper -- if that is the shared admin credential, this push is NOT running as '$agent'" >&2
    fi
    exit 0   # exit 2 case only: no App provisioned, nothing to say
fi

printf 'protocol=%s\n' "${protocol:-https}"
printf 'host=%s\n' "$host"
printf 'username=x-access-token\n'
printf 'password=%s\n' "$token"
