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
TARGET_REPO=""
if REMOTE_URL="$(git remote get-url origin 2>/dev/null)"; then
    # Two patterns rather than one with an optional `.git`: bash uses POSIX
    # ERE, which has no non-greedy quantifier, so `([^/]+?)(\.git)?$` does
    # not match at all. It failing SILENTLY is the dangerous part - TARGET_ORG
    # just kept its default and we minted a token for the wrong account,
    # which then 403s on write while still passing a public-repo read probe.
    if [[ "$REMOTE_URL" =~ [:/]([^/:]+)/([^/]+)\.git$ ]] \
       || [[ "$REMOTE_URL" =~ [:/]([^/:]+)/([^/]+)$ ]]; then
        TARGET_ORG="${BASH_REMATCH[1]}"
        TARGET_REPO="${BASH_REMATCH[2]}"
    fi
fi

# The remote is only a fallback. An installation token is scoped to ONE
# account, so deriving the target purely from the current directory breaks
# every cross-org call: `gh api repos/a5af/dev-tools/...` run from inside the
# agentmuxai checkout mints an agentmuxai token and gets a bare 404, which
# reads as "that repo doesn't exist" rather than "wrong account". A PAT had no
# such constraint, so this would be a straight regression if left unhandled -
# and it's the thing blocking removal of the PAT tier entirely.
#
# So when the caller names an owner explicitly, that wins over the directory.
#
# Both matchers below are deliberately narrow. Scanning all of argv for
# anything shaped like an owner is the tempting version and it is wrong: a
# branch name, title or field value can take any shape a real reference can.
# `--head security/some-fix` looks like OWNER/REPO; `--head orgs/migrate-team`
# looks like an orgs path; `--head repos/some-feature/sub` looks like a repos
# path. Each would mint a token for an account that does not exist and turn a
# working command into an auth failure - the same class of bug this whole
# block exists to remove. So position is what qualifies a match, not shape.

# (a) -R/--repo takes an OWNER/REPO value, in any subcommand.
for ((i = 1; i <= $#; i++)); do
    arg="${!i}"
    case "$arg" in
        -R|--repo)
            next_i=$((i + 1))
            [[ $next_i -le $# ]] || continue
            arg="${!next_i}"
            ;;
        --repo=*) arg="${arg#--repo=}" ;;
        # Attached shorthand (`-Rowner/repo`). gh is cobra/pflag-based and
        # accepts it, so it must be handled or the explicit owner is ignored.
        -R?*)     arg="${arg#-R}" ;;
        *) continue ;;
    esac
    if [[ "$arg" =~ ^([A-Za-z0-9._-]+)/([A-Za-z0-9._-]+)$ ]]; then
        TARGET_ORG="${BASH_REMATCH[1]}"; TARGET_REPO="${BASH_REMATCH[2]}"
        EXPLICIT_TARGET=1
    fi
    break
done

# (b) `gh api <endpoint>` - ONLY the endpoint, which is the first positional
# argument after the `api` subcommand. Flags that take a separate value must
# be stepped over so their value is never mistaken for the endpoint.
if [[ -z "${EXPLICIT_TARGET:-}" && "${1:-}" == "api" ]]; then
    skip_next=0
    for ((i = 2; i <= $#; i++)); do
        arg="${!i}"
        if [[ $skip_next -eq 1 ]]; then skip_next=0; continue; fi
        case "$arg" in
            -X|--method|-F|--field|-f|--raw-field|-H|--header|--input|--jq|-q|--template|-t|--cache|--hostname)
                skip_next=1; continue ;;
            -*) continue ;;
        esac
        # First positional wins; the leading slash is optional because `gh api`
        # documents and accepts both `repos/O/R/...` and `/repos/O/R/...`.
        if [[ "$arg" =~ ^/?repos/([^/]+)/([^/]+) ]]; then
            TARGET_ORG="${BASH_REMATCH[1]}"; TARGET_REPO="${BASH_REMATCH[2]}"
        elif [[ "$arg" =~ ^/?orgs/([^/]+) ]]; then
            # Org-scoped: leave TARGET_REPO empty so the reachability probe is
            # skipped - there is no repo to probe.
            TARGET_ORG="${BASH_REMATCH[1]}"; TARGET_REPO=""
        fi
        break
    done
fi

TOKEN=""
USED_KEY=""
AUTH_MODE=""

# Minting a token successfully is NOT the same as that token being able to
# reach the repo we're about to operate on: an App installed on one account
# mints perfectly valid tokens that return 403 "Resource not accessible by
# integration" against a repo in a different org. That failure cost a long
# debugging session precisely because it looks like a permissions bug on the
# App rather than a missing installation.
#
# So each tier is probed before it's accepted. Read what this probe does and
# does not tell you, because the distinction is not academic:
#
#   IT DOES detect "this token has no access to this repo at all" - the
#   wrong-org-installation case above, which is the one that actually bit.
#
#   IT DOES NOT verify the token can perform the specific operation the
#   caller is about to run. Demonstrated on this very repo: agenty's App
#   passes this probe with 200, and still gets 403 creating a PR comment,
#   despite its token carrying issues:write. Reachability and capability are
#   different questions, and only the caller knows which capability it needs.
#
# A capability probe is not possible here - we cannot know the operation, and
# any write-shaped probe would have side effects. So the probe stays scoped
# to reachability, and the gap is covered at the other end instead: if gh
# itself fails while we're on an App token, we annotate the failure with the
# likely cause rather than leaving an opaque 403 (see the tail of this file).
#
# The probe runs BEFORE the caller's command, so falling through to the next
# tier can never re-run a mutation that already partially succeeded. If the
# repo can't be determined (outside a checkout) or curl is unavailable, the
# probe is skipped rather than treated as a failure.
token_reaches_target() {
    local token="$1"
    [[ -z "$TARGET_REPO" ]] && return 0
    command -v curl >/dev/null 2>&1 || return 0
    local code
    # The token goes in via --config on stdin, never as an argv element:
    # this machine runs many agents under one OS user, and argv is readable
    # from the process table by anything else running here.
    code="$(printf 'header = "Authorization: Bearer %s"
header = "Accept: application/vnd.github+json"
silent
output = "/dev/null"
write-out = "%%{http_code}"
max-time = 10
url = "https://api.github.com/repos/%s/%s"
'         "$token" "$TARGET_ORG" "$TARGET_REPO" | curl --config - 2>/dev/null)" || return 0
    [[ "$code" == "200" ]]
}

# github-app-token.py exit codes are the contract here, not its stderr text:
#   2 = this agent has no App identity provisioned (expected, quiet)
#   1 = an identity exists but minting genuinely failed (surface it)
try_app_tier() {
    local identity="$1" keyname="$2" label="$3"
    command -v python3 >/dev/null 2>&1 || return 1
    local err_file token rc
    err_file="$(mktemp)"
    token="$(python3 "$SCRIPT_DIR/github-app-token.py" "$identity" "$TARGET_ORG" 2>"$err_file")"
    rc=$?
    if [[ $rc -eq 0 ]]; then
        if token_reaches_target "$token"; then
            TOKEN="$token"; USED_KEY="$keyname"; AUTH_MODE="app"
            rm -f "$err_file"; return 0
        fi
        echo "gh-agent: $label token minted but cannot reach ${TARGET_ORG}/${TARGET_REPO}" >&2
        echo "          (App likely not installed on '${TARGET_ORG}') - trying next tier" >&2
    elif [[ $rc -ne 2 ]]; then
        echo "gh-agent: $label token mint failed - trying next tier:" >&2
        cat "$err_file" >&2
    fi
    rm -f "$err_file"
    return 1
}

# --- Tier 1: this agent's own GitHub App installation token ----------------
try_app_tier "$AGENT_LOWER" "${AGENT_LOWER}-workflow-key" "own App" || true

# --- Tier 2: shared genericagentx-workflow App installation token ----------
if [[ -z "$TOKEN" ]]; then
    try_app_tier genericagentx "genericagentx-workflow-key" "shared App" || true
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

# Deliberately not `exec`: we want the exit code back so a failure can be
# annotated. This is NOT a retry - the command runs exactly once, and we only
# add a diagnostic line afterwards. Retrying under a different identity could
# double-execute a mutation that already partially succeeded.
# `|| GH_RC=$?` rather than a bare call: this script runs under `set -e`, so a
# failing gh would abort here and skip the diagnostic below - which is exactly
# the case the diagnostic exists for. A command in a `||` list is exempt from
# errexit.
GH_RC=0
GH_TOKEN="$TOKEN" gh "$@" || GH_RC=$?

if [[ $GH_RC -ne 0 && "$AUTH_MODE" == "app" ]]; then
    {
        echo "gh-agent: that failed while authenticated as a GitHub App"
        echo "          ($USED_KEY). If the error mentioned \"Resource not"
        echo "          accessible by integration\", the App reached the repo"
        echo "          but lacks permission for THAT operation - the tier"
        echo "          probe only checks reachability, not per-operation"
        echo "          capability. Check the App's granted permissions for"
        echo "          '${TARGET_ORG}', and note an installation must"
        echo "          separately accept a permission the App definition"
        echo "          added later."
    } >&2
fi

exit $GH_RC
