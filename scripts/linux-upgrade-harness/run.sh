#!/usr/bin/env bash
# Real-install harness for scripts/agentmux-install-update.sh on .deb and .rpm distros.
#
#   ./run.sh [distro...]          run every case on each distro (default: ubuntu24 ubuntu26 fedora42)
#   ./run.sh -c <case> [distro]   one case
#   ./run.sh shell <distro>       a root shell in a container with AgentMux $FROM installed, on the mirror network
#   ./run.sh mirror-up | mirror-down
#   ./run.sh clean                remove what this harness created, by recorded id (cache kept; --all drops it too)
#
# Distros: ubuntu24 ubuntu26 fedora42 debian13 tumbleweed
# Env: FROM_VERSION (3-part, default 0.60.3), TO_VERSION (0.60.4), HELPER (a helper to test instead of the one
#      the package installed: a path on this host, mounted in), KEEP=1 (keep snapshot images between runs).
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"
FROM_VERSION="${FROM_VERSION:-0.60.3}"
TO_VERSION="${TO_VERSION:-0.60.4}"
STATE="$HERE/state"; CACHE="${CACHE:-$HERE/cache}"; WWW="$HERE/www"; CERTS="$STATE/certs"
# HARNESS_ID: a second user of a copy of this directory sets its own, so the network, mirror and images differ.
IMG="amx-harness${HARNESS_ID:+-$HARNESS_ID}"
NET="$IMG"
MIRROR_NAME="$IMG-mirror"
MIRROR_HOST=mirror.amx-harness.test
LABEL="amx.harness=${HARNESS_ID:-maricon-2}"
mkdir -p "$STATE" "$CACHE" "$WWW/api" "$WWW/blob"
touch "$STATE/containers" "$STATE/images"

base_of() { case "$1" in
    ubuntu24) echo "ubuntu:24.04 deb";; ubuntu26) echo "ubuntu:26.04 deb";; debian13) echo "debian:13 deb";;
    fedora42) echo "fedora:42 dnf";; tumbleweed) echo "opensuse/tumbleweed zypper";;
    *) echo "unknown distro $1" >&2; return 1;; esac; }
pkg_of() { # distro version
    read -r _ fam <<<"$(base_of "$1")"
    if [ "$fam" = deb ]; then echo "AgentMux_$2_amd64.deb"; else echo "AgentMux-$2-1.x86_64.rpm"; fi
}

record() { echo "$2" >>"$STATE/$1"; }
forget() { grep -vxF "$2" "$STATE/$1" >"$STATE/$1.tmp" 2>/dev/null; mv "$STATE/$1.tmp" "$STATE/$1"; }

fetch_packages() {
    local v f
    for v in "$FROM_VERSION" "$TO_VERSION"; do
        for f in "AgentMux_${v}_amd64.deb" "AgentMux-${v}-1.x86_64.rpm"; do
            [ -s "$CACHE/$f" ] || curl -fsSL -o "$CACHE/$f" "https://github.com/agentmuxai/agentmux/releases/download/v$v/$f" || { echo "can't download $f" >&2; return 1; }
        done
    done
}

make_certs() {
    [ -s "$CERTS/server.crt" ] && return 0
    mkdir -p "$CERTS" && cd "$CERTS" || return 1
    openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=amx-harness-ca" -keyout ca.key -out ca.crt 2>/dev/null
    openssl req -newkey rsa:2048 -nodes -subj "/CN=$MIRROR_HOST" -keyout server.key -out server.csr 2>/dev/null
    printf 'subjectAltName=DNS:%s\nbasicConstraints=CA:FALSE\n' "$MIRROR_HOST" >san.ext
    openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -days 30 -extfile san.ext -out server.crt 2>/dev/null
    cd "$HERE"; cp "$CERTS/ca.crt" ctx/ca.crt
}

# The real releases, as the mirror lists them: GitHub's pretty-printed listing, one entry per file.
publish_real() {
    local v f first
    for v in "$FROM_VERSION" "$TO_VERSION"; do
        mkdir -p "$WWW/blob/v$v"
        first=1
        {
            printf '{\n  "tag_name": "v%s",\n  "assets": [\n' "$v"
            for f in "AgentMux_${v}_amd64.deb" "AgentMux-${v}-1.x86_64.rpm"; do
                ln -f "$CACHE/$f" "$WWW/blob/v$v/$f" 2>/dev/null || cp "$CACHE/$f" "$WWW/blob/v$v/$f"
                [ "$first" = 1 ] || printf '    ,\n'; first=0
                printf '    {\n      "id": 1,\n      "name": "%s",\n      "size": %s,\n      "digest": "sha256:%s"\n    }\n' "$f" "$(stat -c %s "$CACHE/$f")" "$(sha256sum <"$CACHE/$f" | awk '{print $1}')"
            done
            printf '  ]\n}\n'
        } >"$WWW/api/v$v"
    done
}

mirror_up() {
    make_certs || return 1
    docker network inspect "$NET" >/dev/null 2>&1 || { docker network create --label "$LABEL" "$NET" >/dev/null && record network "$NET"; }
    if [ -n "$(docker ps -q -f name="^$MIRROR_NAME$")" ]; then return 0; fi
    docker rm -f "$MIRROR_NAME" >/dev/null 2>&1
    publish_real
    local id
    id=$(docker run -d --name "$MIRROR_NAME" --label "$LABEL" --network "$NET" --network-alias "$MIRROR_HOST" \
        -v "$HERE/mirror/mirror.py:/mirror.py:ro" -v "$WWW:/www" -v "$CERTS:/certs:ro" python:3.13-slim python /mirror.py) || return 1
    record containers "$id"
    for _ in $(seq 1 30); do docker logs "$MIRROR_NAME" 2>&1 | grep -q 'mirror up' && return 0; sleep 1; done
    echo "mirror didn't start" >&2; docker logs "$MIRROR_NAME" >&2; return 1
}
mirror_down() {
    local id
    while read -r id; do [ -n "$id" ] && docker rm -f "$id" >/dev/null 2>&1; forget containers "$id"; done <"$STATE/containers"
}

# The image of <distro> with AgentMux $FROM_VERSION installed by its own package manager.
snapshot() { # distro
    local d=$1 base fam img="$IMG/$1:$FROM_VERSION" id
    read -r base fam <<<"$(base_of "$d")" || return 1
    if [ -n "$(docker images -q "$img")" ] && [ "${REBUILD:-0}" != 1 ]; then return 0; fi
    echo "== $d: building the image on $base" >&2
    docker build -q -t "$IMG/$d-tools" --label "$LABEL" --build-arg "BASE=$base" -f "ctx/Dockerfile.$fam" ctx >/dev/null || return 1
    record images "$IMG/$d-tools"
    local install
    case "$fam" in
        deb) install="DEBIAN_FRONTEND=noninteractive apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends /pkgs/$(pkg_of "$d" "$FROM_VERSION")";;
        dnf) install="dnf -y install /pkgs/$(pkg_of "$d" "$FROM_VERSION")";;
        zypper) install="zypper --non-interactive install --allow-unsigned-rpm /pkgs/$(pkg_of "$d" "$FROM_VERSION")";;
    esac
    id=$(docker create --label "$LABEL" --network "$NET" -v "$CACHE:/pkgs:ro" "$IMG/$d-tools" bash -c "$install") || return 1
    record containers "$id"
    docker start -a "$id" >"$STATE/$d-install.log" 2>&1 || { echo "installing $FROM_VERSION on $d failed; see $STATE/$d-install.log" >&2; docker rm -f "$id" >/dev/null; forget containers "$id"; return 1; }
    docker commit "$id" "$img" >/dev/null && record images "$img"
    docker rm -f "$id" >/dev/null; forget containers "$id"
}

run_case() { # distro case
    local d=$1 c=$2 id rc helper_mount=() helper_env=() waiter_mount=()
    local launcher_src="$HERE/../../crates/launcher/src/upgrade_package.rs"
    [ -f "$launcher_src" ] && waiter_mount=(-v "$(readlink -f "$launcher_src"):/upgrade_package.rs:ro")
    if [ -n "${HELPER:-}" ]; then helper_mount=(-v "$(readlink -f "$HELPER"):/helper-under-test:ro"); helper_env=(-e HELPER=/helper-under-test); fi
    # SYS_PTRACE: a real host's root can read another user's /proc/<pid>/exe, which the helper's
    # "is anything running from /opt/agentmux" check needs; Docker's default root cannot.
    id=$(docker create --cap-add SYS_PTRACE --label "$LABEL" --network "$NET" -e "FROM_VERSION=$FROM_VERSION" -e "TO_VERSION=$TO_VERSION" "${helper_env[@]}" \
        -v "$CACHE:/pkgs:ro" -v "$WWW:/www" -v "$HERE/cases.sh:/cases.sh:ro" "${waiter_mount[@]}" "${helper_mount[@]}" "$IMG/$d:$FROM_VERSION" bash /cases.sh "$c") || return 2
    record containers "$id"
    docker start -a "$id"; rc=$?
    docker rm -f "$id" >/dev/null 2>&1; forget containers "$id"
    return $rc
}

case_list() { grep -o '^case_[a-z_0-9]*' "$HERE/cases.sh" | sed 's/^case_//'; }

shell_in() {
    snapshot "$1" || return 1; mirror_up || return 1
    local id; id=$(docker create -it --cap-add SYS_PTRACE --label "$LABEL" --network "$NET" -e "FROM_VERSION=$FROM_VERSION" -e "TO_VERSION=$TO_VERSION" \
        -v "$CACHE:/pkgs:ro" -v "$WWW:/www" -v "$HERE/cases.sh:/cases.sh:ro" "$IMG/$1:$FROM_VERSION" bash) || return 1
    record containers "$id"; docker start -ai "$id"; docker rm -f "$id" >/dev/null 2>&1; forget containers "$id"
}

clean() {
    local x
    mirror_down
    while read -r x; do [ -n "$x" ] && docker rm -f "$x" >/dev/null 2>&1; done <"$STATE/containers"; : >"$STATE/containers"
    while read -r x; do [ -n "$x" ] && docker rmi -f "$x" >/dev/null 2>&1; done <"$STATE/images"; : >"$STATE/images"
    while read -r x; do [ -n "$x" ] && docker network rm "$x" >/dev/null 2>&1; done <"$STATE/network" 2>/dev/null; rm -f "$STATE/network"
    rm -rf "$WWW"/api/* "$WWW"/blob/*
    if [ "${1:-}" = --all ]; then rm -rf "$CACHE" "$STATE/certs" "$HERE/ctx/ca.crt"; fi
    echo "cleaned"
}

main() {
    local only="" distros=()
    case "${1:-}" in
        clean) shift; clean "$@"; return;;
        mirror-up) mirror_up; return;;
        mirror-down) mirror_down; return;;
        shell) shell_in "${2:-ubuntu24}"; return;;
        -c) only=$2; shift 2;;
    esac
    distros=("$@"); [ ${#distros[@]} -gt 0 ] || distros=(ubuntu24 ubuntu26 fedora42)
    fetch_packages || return 1
    mirror_up || return 1
    local d c failed=0 total=0 cases=()
    if [ -n "$only" ]; then cases=("$only"); else mapfile -t cases < <(case_list); fi
    for d in "${distros[@]}"; do
        snapshot "$d" || { echo "$d: SETUP FAILED"; failed=$((failed+1)); continue; }
        echo "== $d"
        for c in "${cases[@]}"; do
            total=$((total+1))
            printf '%s\n' "- $c"
            run_case "$d" "$c" || failed=$((failed+1))
        done
    done
    echo "== $total cases, $failed failed"
    [ "${KEEP:-0}" = 1 ] || true
    [ "$failed" = 0 ]
}
main "$@"
