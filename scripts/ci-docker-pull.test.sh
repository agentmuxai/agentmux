#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Test scripts/ci-docker-pull.sh with a stand-in `docker`: no network, no daemon.
#
#   scripts/ci-docker-pull.test.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The stand-in: `docker pull <source>` fails for every source listed in $FAIL (space separated) and
# succeeds for the rest; every call is appended to $LOG.
mkdir -p "$WORK/bin"
cat >"$WORK/bin/docker" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$LOG"
if [ "$1" = pull ]; then
    for s in $FAIL; do [ "${*: -1}" = "$s" ] && exit 1; done
fi
exit 0
EOF
chmod +x "$WORK/bin/docker"
export PATH="$WORK/bin:$PATH" LOG="$WORK/log" CI_DOCKER_PULL_WAIT=0 CI_DOCKER_PULL_TRIES=2

fail=0
check() { # name, expected, actual
    if [ "$2" != "$3" ]; then echo "FAIL: $1"; echo "  expected: $2"; echo "  actual:   $3"; fail=1; else echo "ok: $1"; fi
}
calls() { tr '\n' ';' <"$LOG"; }

# An official image comes from Amazon's mirror first, and is tagged with the name it was asked by.
: >"$LOG"; FAIL="" bash "$HERE/ci-docker-pull.sh" ubuntu:24.04 >/dev/null
check "an official image: the Amazon mirror, then a tag" \
    "pull --quiet public.ecr.aws/docker/library/ubuntu:24.04;tag public.ecr.aws/docker/library/ubuntu:24.04 ubuntu:24.04;" "$(calls)"

# A source that fails is tried twice, then the next source answers.
: >"$LOG"; FAIL="public.ecr.aws/docker/library/fedora:42" bash "$HERE/ci-docker-pull.sh" fedora:42 >/dev/null 2>&1
check "a refused source is retried, then the next one is used" \
    "pull --quiet public.ecr.aws/docker/library/fedora:42;pull --quiet public.ecr.aws/docker/library/fedora:42;pull --quiet mirror.gcr.io/library/fedora:42;tag mirror.gcr.io/library/fedora:42 fedora:42;" "$(calls)"

# An organisation's image keeps its path; openSUSE's own registry comes first.
: >"$LOG"; FAIL="registry.opensuse.org/opensuse/tumbleweed" bash "$HERE/ci-docker-pull.sh" opensuse/tumbleweed >/dev/null 2>&1
check "an organisation's image: its own registry, then the Google mirror" \
    "pull --quiet registry.opensuse.org/opensuse/tumbleweed;pull --quiet registry.opensuse.org/opensuse/tumbleweed;pull --quiet mirror.gcr.io/opensuse/tumbleweed;tag mirror.gcr.io/opensuse/tumbleweed opensuse/tumbleweed;" "$(calls)"

# Docker Hub is the last resort, and when nothing answers the script fails and says so.
: >"$LOG"
if FAIL="public.ecr.aws/docker/library/debian:13 mirror.gcr.io/library/debian:13 docker.io/library/debian:13" bash "$HERE/ci-docker-pull.sh" debian:13 >/dev/null 2>"$WORK/err"; then
    echo "FAIL: nothing answers, yet the script succeeded"; fail=1
else
    echo "ok: nothing answers: the script fails"
fi
check "Docker Hub is tried last" "docker.io/library/debian:13" "$(grep -o 'docker\.io/library/debian:13' "$LOG" | tail -1)"
grep -q "no source would give debian:13" "$WORK/err" && echo "ok: it says which image" || { echo "FAIL: no message for debian:13"; fail=1; }

# Every image of a call is pulled, and one that cannot be fails the call after the others were tried.
: >"$LOG"
if FAIL="public.ecr.aws/docker/library/ubuntu:24.04 mirror.gcr.io/library/ubuntu:24.04 docker.io/library/ubuntu:24.04" bash "$HERE/ci-docker-pull.sh" ubuntu:24.04 fedora:42 >/dev/null 2>&1; then
    echo "FAIL: one image failed, yet the script succeeded"; fail=1
else
    echo "ok: one image fails the call"
fi
grep -q "tag public.ecr.aws/docker/library/fedora:42 fedora:42" "$LOG" && echo "ok: the next image was still pulled" || { echo "FAIL: fedora was not pulled"; fail=1; }

exit "$fail"
