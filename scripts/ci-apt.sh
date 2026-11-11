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
#   2. a wait for the Ubuntu archive hosts to resolve before the first attempt (CI_APT_DNS_WAIT, 300 s), so a
#      short outage costs a pause, not three timed-out attempts;
#   3. the whole command again, up to CI_APT_ATTEMPTS (3) times with a growing pause, each attempt bounded by
#      `timeout` (CI_APT_UPDATE_LIMIT 180 s, CI_APT_INSTALL_LIMIT 300 s).
#
# `update` succeeds when no Ubuntu archive host failed in its log; a broken THIRD-PARTY index and apt's exit code
# alone do not count (a release must never build from stale Ubuntu lists, but a corrupt Google index must not stop it).
# Without --strict, running out of attempts only warns, as the PR leg always has: its `install` is the real gate.
# `install` exits non-zero when it never succeeded.
#
# A network that stays down for longer than all of this still fails the job, in minutes instead of a quarter hour.
# Nothing here removes the network from the path.

set -uo pipefail

ATTEMPTS=${CI_APT_ATTEMPTS:-3}
PAUSE=${CI_APT_PAUSE:-45}
DNS_WAIT=${CI_APT_DNS_WAIT:-300}
UPDATE_LIMIT=${CI_APT_UPDATE_LIMIT:-180}
INSTALL_LIMIT=${CI_APT_INSTALL_LIMIT:-300}
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

wait_for_dns() {
    local waited=0
    while ! resolves; do
        if [ "$waited" -ge "$DNS_WAIT" ]; then
            log "the Ubuntu archive hosts still do not resolve after ${waited}s; trying apt anyway"
            { grep -v '^#' /etc/resolv.conf 2>/dev/null | head -5; for h in $HOSTS; do getent hosts "$h" || echo "no answer for $h"; done; } | sed 's/^/ci-apt:   /'
            return 0
        fi
        log "the Ubuntu archive hosts do not resolve yet; waiting (${waited}s of ${DNS_WAIT}s)"
        sleep 15
        waited=$((waited + 15))
    done
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
    wait_for_dns
    local out
    out=$(mktemp)
    local n
    for n in $(seq 1 "$ATTEMPTS"); do
        [ "$n" -eq 1 ] || pause_before "$n"
        if update_once "$out"; then return 0; fi
    done
    if [ "$strict" -eq 1 ]; then
        echo "::error::apt-get update failed against the Ubuntu archive after $ATTEMPTS attempts - refusing to build from stale package lists"
        return 1
    fi
    log "apt-get update still reports errors after $ATTEMPTS attempts; continuing (install is the real gate)"
}

cmd_install() {
    [ "$#" -gt 0 ] || { echo "usage: ci-apt.sh install <apt-get install arguments...>" >&2; return 2; }
    configure
    wait_for_dns
    local n
    for n in $(seq 1 "$ATTEMPTS"); do
        if [ "$n" -gt 1 ]; then
            pause_before "$n"
            # Lists that were stale or half fetched are the other way an install fails twice.
            sudo timeout "$UPDATE_LIMIT" apt-get update >/dev/null 2>&1 || true
        fi
        sudo timeout "$INSTALL_LIMIT" apt-get install "$@" && return 0
    done
    echo "::error::apt-get install $* failed after $ATTEMPTS attempts"
    return 1
}

case "${1:-}" in
    update) shift; cmd_update "$@" ;;
    install) shift; cmd_install "$@" ;;
    *) echo "usage: ci-apt.sh update [--strict] | install <apt-get install arguments...>" >&2; exit 2 ;;
esac
