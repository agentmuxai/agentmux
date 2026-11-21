#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Test scripts/docker-base-digests.sh with a stand-in `curl` and a copy of the Dockerfiles: no network.
#
#   scripts/docker-base-digests.test.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

OLD_R="sha256:$(printf 'a%.0s' {1..64})"
OLD_N="sha256:$(printf 'b%.0s' {1..64})"
NEW_R="sha256:$(printf 'c%.0s' {1..64})"

# A copy of the script's tree: scripts/ and a docker/ with two Dockerfiles that pin both images.
mkdir -p "$WORK/tree/scripts" "$WORK/tree/docker" "$WORK/bin" "$WORK/digests"
cp "$HERE/docker-base-digests.sh" "$WORK/tree/scripts/"
for f in Dockerfile.srv Dockerfile.agent; do
    printf 'ARG RUST_IMAGE=public.ecr.aws/docker/library/rust:1-bookworm@%s\nARG NODE_IMAGE=public.ecr.aws/docker/library/node:24-slim@%s\nARG OTHER=ghcr.io/x/y:1@%s\nFROM ${RUST_IMAGE}\n' "$OLD_R" "$OLD_N" "$OLD_N" >"$WORK/tree/docker/$f"
done

# The stand-in: the token endpoint answers a token; a manifest request answers the digest in $DIGESTS/<name>.
cat >"$WORK/bin/curl" <<'EOF'
#!/usr/bin/env bash
url="${*: -1}"
case "$url" in
    */token/) echo '{"token":"t0k3n"}' ;;
    */v2/docker/library/*/manifests/*)
        name="${url#*/library/}"; name="${name%%/*}"
        [ -f "$DIGESTS/$name" ] || exit 22
        printf 'HTTP/2 200\r\ncontent-type: x\r\nDocker-Content-Digest: %s\r\n\r\n' "$(cat "$DIGESTS/$name")" ;;
    *) exit 22 ;;
esac
EOF
chmod +x "$WORK/bin/curl"
export PATH="$WORK/bin:$PATH" DIGESTS="$WORK/digests"

fail=0
check() { # name, expected, actual
    if [ "$2" != "$3" ]; then echo "FAIL: $1"; echo "  expected: $2"; echo "  actual:   $3"; fail=1; else echo "ok: $1"; fi
}
run() { set +e; out="$(bash "$WORK/tree/scripts/docker-base-digests.sh" "$@" 2>&1)"; code=$?; set -e; }

# Nothing moved: exit 0, nothing said.
echo "$OLD_R" >"$DIGESTS/rust"; echo "$OLD_N" >"$DIGESTS/node"
run --check
check "pins that are current: exit 0" "0" "$code"
check "pins that are current: nothing said" "" "$out"

# The rust tag moved: exit 1, both Dockerfiles named with the old and the new digest.
echo "$NEW_R" >"$DIGESTS/rust"
run --check
check "a moved tag: exit 1" "1" "$code"
check "a moved tag: each Dockerfile is named" "2" "$(grep -c 'RUST_IMAGE (docker/library/rust:1-bookworm) moved' <<<"$out")"
grep -q "pinned  $OLD_R" <<<"$out" && grep -q "current $NEW_R" <<<"$out" && echo "ok: the old and the new digest are shown" || { echo "FAIL: digests not shown: $out"; fail=1; }
check "a check changes no file" "$OLD_R" "$(sed -n 's/^ARG RUST_IMAGE=.*@//p' "$WORK/tree/docker/Dockerfile.srv")"

# --update moves only the moved pin, in both files, and leaves the other registry's pin alone.
run --update
check "update: exit 0" "0" "$code"
for f in Dockerfile.srv Dockerfile.agent; do
    check "update: $f rust pin" "$NEW_R" "$(sed -n 's/^ARG RUST_IMAGE=.*@//p' "$WORK/tree/docker/$f")"
    check "update: $f node pin kept" "$OLD_N" "$(sed -n 's/^ARG NODE_IMAGE=.*@//p' "$WORK/tree/docker/$f")"
    check "update: $f other registry kept" "$OLD_N" "$(sed -n 's/^ARG OTHER=.*@//p' "$WORK/tree/docker/$f")"
done
run --check
check "after an update the check passes" "0" "$code"

# A registry that cannot be asked: exit 2, not 0.
rm "$DIGESTS/node"
run --check
check "an unanswered lookup: exit 2" "2" "$code"
grep -q "could not read the digest" <<<"$out" && echo "ok: it says which image" || { echo "FAIL: no message: $out"; fail=1; }

run --nonsense
check "a bad option: exit 2" "2" "$code"

exit "$fail"
