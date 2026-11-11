#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# apt for CI runners that survives a short network or resolver failure.
#
#   scripts/ci-apt.sh update [--strict]     refresh the package lists
#   scripts/ci-apt.sh install <apt-get install arguments...>
#
# Three layers, because one apt run with its defaults spent 11 minutes on a dead resolver and then failed:
#   1. apt's own retries and a short per-file timeout (/etc/apt/apt.conf.d/80-ci-retries);
#   2. a wait for the Ubuntu archive hosts to resolve before anything else (CI_APT_DNS_WAIT, 300 s; 0 skips the
#      check). A short outage costs a pause, not three timed-out attempts; a longer one fails the step right there
#      with the resolver state printed, instead of 11 minutes later;
#   3. the whole command again, up to CI_APT_ATTEMPTS (3) times with a growing pause, each attempt bounded by
#      `timeout` (CI_APT_UPDATE_LIMIT 180 s, CI_APT_INSTALL_LIMIT 300 s), and no new attempt once the call has
#      used CI_APT_BUDGET (600 s).
#
# `update` succeeds when no Ubuntu archive host failed in its log; a broken THIRD-PARTY index and apt's exit code
# alone do not count (a release must never build from stale Ubuntu lists, but a corrupt Google index must not stop it).
# Without --strict, running out of attempts only warns, as the PR leg always has: its `install` is the real gate.
# `install` exits non-zero when it never succeeded. Nothing here removes the network from the path.

set -uo pipefail

ATTEMPTS=${CI_APT_ATTEMPTS:-3}
PAUSE=${CI_APT_PAUSE:-45}
DNS_WAIT=${CI_APT_DNS_WAIT:-300}
UPDATE_LIMIT=${CI_APT_UPDATE_LIMIT:-180}
INSTALL_LIMIT=${CI_APT_INSTALL_LIMIT:-300}
BUDGET=${CI_APT_BUDGET:-600}
HOSTS="archive.ubuntu.com security.ubuntu.com"
# The error form and the warning form: `-qq` turns an `Err:<n> <url>` record into `W: Failed to fetch ...`.
UBUNTU_FAILED='^(E|Err):.*(archive\.ubuntu\.com|security\.ubuntu\.com)|^W: Failed to fetch.*(archive\.ubuntu\.com|security\.ubuntu\.com)'

log() { echo "ci-apt: $*"; }

configure() {
    printf 'Acquire::Retries "5";\nAcquire::http::Timeout "20";\nAcquire::https::Timeout "20";\n' \
        | sudo tee /etc/apt/apt.conf.d/80-ci-retries >/dev/null
}

resolves() {
    local h
    for h in $HOSTS; do getent hosts "$h" >/dev/null 2>&1 || return 1; done
}

# 0 when the Ubuntu archive hosts resolve (waiting up to DNS_WAIT for them to), 1 when they never did.
wait_for_dns() {
    [ "$DNS_WAIT" -gt 0 ] || return 0
    local waited=0 h
    while ! resolves; do
        if [ "$waited" -ge "$DNS_WAIT" ]; then
            echo "::error::the Ubuntu archive hosts still do not resolve after ${waited}s: this runner has no name resolution for apt"
            { grep -v '^#' /etc/resolv.conf 2>/dev/null | head -5; for h in $HOSTS; do getent hosts "$h" || echo "no answer for $h"; done; } | sed 's/^/ci-apt:   /'
            return 1
        fi
        log "the Ubuntu archive hosts do not resolve yet; waiting (${waited}s of ${DNS_WAIT}s)"
        sleep 15
        waited=$((waited + 15))
    done
}

# Whether attempt $1 may start: attempts left, and the call's budget not spent by the attempt's own pause.
may_try() {
    local next=$1 started=$2
    [ "$next" -le "$ATTEMPTS" ] || return 1
    if [ $((SECONDS - started + PAUSE * (next - 1))) -ge "$BUDGET" ]; then
        log "not trying again: the ${BUDGET}s budget for this call is used up"
        return 1
    fi
}

pause_before() {
    local next=$1
    log "attempt $((next - 1)) failed; waiting $((PAUSE * (next - 1)))s before attempt $next"
    sleep $((PAUSE * (next - 1)))
}

update_once() {
    local out=$1
    # No -qq: it hides the `Err:` records the check below reads.
    sudo timeout "$UPDATE_LIMIT" apt-get update 2>&1 | tee "$out"
    local status=${PIPESTATUS[0]}
    [ "$status" -ne 124 ] || { log "apt-get update ran past ${UPDATE_LIMIT}s"; return 1; }
    ! grep -qE "$UBUNTU_FAILED" "$out"
}

cmd_update() {
    local strict=0
    [ "${1:-}" != "--strict" ] || strict=1
    configure
    wait_for_dns || return 1
    local out started=$SECONDS n=1
    out=$(mktemp)
    while :; do
        if update_once "$out"; then return 0; fi
        n=$((n + 1))
        may_try "$n" "$started" || break
        pause_before "$n"
    done
    if [ "$strict" -eq 1 ]; then
        echo "::error::apt-get update failed against the Ubuntu archive after $((n - 1)) attempt(s) - refusing to build from stale package lists"
        return 1
    fi
    log "apt-get update still reports errors after $((n - 1)) attempt(s); continuing (install is the real gate)"
}

cmd_install() {
    [ "$#" -gt 0 ] || { echo "usage: ci-apt.sh install <apt-get install arguments...>" >&2; return 2; }
    configure
    wait_for_dns || return 1
    local started=$SECONDS n=1
    while :; do
        sudo timeout "$INSTALL_LIMIT" apt-get install "$@" && return 0
        n=$((n + 1))
        may_try "$n" "$started" || break
        pause_before "$n"
        # Lists that were stale or half fetched are the other way an install fails twice.
        sudo timeout "$UPDATE_LIMIT" apt-get update >/dev/null 2>&1 || true
    done
    echo "::error::apt-get install $* failed after $((n - 1)) attempt(s)"
    return 1
}

case "${1:-}" in
    update) shift; cmd_update "$@" ;;
    install) shift; cmd_install "$@" ;;
    *) echo "usage: ci-apt.sh update [--strict] | install <apt-get install arguments...>" >&2; exit 2 ;;
esac
