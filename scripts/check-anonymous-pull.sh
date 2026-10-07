#!/usr/bin/env bash
# check-anonymous-pull.sh -- is a container image readable without credentials?
#
# Usage: bash scripts/check-anonymous-pull.sh <image> [<package-settings-url>]
#
# AgentMux pulls the container agents' base image anonymously (the Docker daemon
# does not use the CLI's `docker login`), so the package must be public. A new
# GHCR package is private by default and only an organization owner can change
# that, in the GitHub UI; this check is what tells them.
#
# Asks the registry the way a pull does: the manifest, then the anonymous bearer
# token if it answers 401, then the manifest again.
#
# Exit 0  the manifest is readable without credentials
# Exit 1  the registry refused (401/403/404): the package is private or missing
# Exit 2  no definite answer (network error, unexpected status)
#
# Spec: docs/specs/SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md section 3.6.

set -uo pipefail

image="${1:-}"
settings_url="${2:-}"
if [ -z "$image" ]; then
    echo "usage: $0 <image> [<package-settings-url>]" >&2
    exit 2
fi

registry="${image%%/*}"
rest="${image#*/}"
repo="${rest%%[:@]*}"
ref="latest"
case "$rest" in
    *@*) ref="${rest#*@}" ;;
    *:*) ref="${rest##*:}" ;;
esac
url="https://$registry/v2/$repo/manifests/$ref"
accept='Accept: application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json'

say() {
    if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::error::$1" >&2; else echo "$1" >&2; fi
}

status_of() {  # status_of [curl args...] -> HTTP status on stdout
    curl -s -o /dev/null -w '%{http_code}' --max-time 30 -H "$accept" "$@" "$url"
}

code="$(status_of)"
if [ "$code" = "401" ]; then
    challenge="$(curl -s -I --max-time 30 -H "$accept" "$url" | tr -d '\r' | grep -i '^www-authenticate:' | head -n 1)"
    realm="$(printf '%s' "$challenge" | sed -n 's/.*realm="\([^"]*\)".*/\1/p')"
    service="$(printf '%s' "$challenge" | sed -n 's/.*service="\([^"]*\)".*/\1/p')"
    if [ -n "$realm" ]; then
        token_url="$realm?scope=repository:$repo:pull"
        [ -n "$service" ] && token_url="$token_url&service=$service"
        body="$(curl -s --max-time 30 "$token_url")"
        token="$(printf '%s' "$body" | sed -n 's/.*"token"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
        [ -z "$token" ] && token="$(printf '%s' "$body" | sed -n 's/.*"access_token"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
        if [ -n "$token" ]; then
            code="$(status_of -H "Authorization: Bearer $token")"
        fi
    fi
fi

case "$code" in
    200)
        echo "anonymous pull of $image works"
        exit 0
        ;;
    401 | 403 | 404)
        msg="$image can't be pulled without credentials (registry answered $code): the package is private or does not exist."
        if [ -n "$settings_url" ]; then
            msg="$msg An organization owner must make it public: open $settings_url, then Danger Zone, Change visibility, Public. Then re-run this workflow."
        fi
        say "$msg"
        exit 1
        ;;
    *)
        say "could not tell whether $image can be pulled without credentials (registry answered '$code'). Re-run the workflow."
        exit 2
        ;;
esac
