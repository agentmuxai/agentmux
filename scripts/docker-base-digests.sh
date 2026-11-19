#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Say whether the base images the shipped Docker images are built on have moved, and move the pins.
#
#   scripts/docker-base-digests.sh --check     exit 0 when every pin is current, 1 and say which moved when not
#   scripts/docker-base-digests.sh --update    rewrite the pins in the Dockerfiles to the current digests
#
# The Dockerfiles in docker/ name each base image by tag and by digest, as an `ARG NAME=registry/path:tag@sha256:...`
# line, so a changed or wrong image behind a tag cannot get into a build unseen. A tag moves on purpose (a security
# update to the image): the pin then needs the new digest, in a reviewed change. This asks the registry for each tag's
# current digest and compares. Only the Amazon public mirror's images are looked at (anonymous, no sign-in); a pin on
# another registry is reported and left alone. Exit 2 when the registry cannot be asked.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODE="${1:---check}"
case "$MODE" in --check | --update) ;; *) echo "usage: docker-base-digests.sh [--check|--update]" >&2; exit 2 ;; esac

REGISTRY=public.ecr.aws
ACCEPT='application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json'

TOKEN="$(curl -fsS "https://$REGISTRY/token/" | sed -n 's/.*"token" *: *"\([^"]*\)".*/\1/p')"
[ -n "$TOKEN" ] || { echo "could not get a token from $REGISTRY" >&2; exit 2; }

# The digest of `path:tag` now, from the registry's answer headers.
current_digest() {
    curl -fsSI -H "Authorization: Bearer $TOKEN" -H "Accept: $ACCEPT" "https://$REGISTRY/v2/$1/manifests/$2" |
        tr -d '\r' | sed -n 's/^[Dd]ocker-[Cc]ontent-[Dd]igest: *//p' | head -1
}

drift=0
for file in "$ROOT"/docker/Dockerfile.*; do
    while read -r name path tag pinned; do
        [ -n "$name" ] || continue
        now="$(current_digest "$path" "$tag" || true)"
        if ! [[ "$now" =~ ^sha256:[0-9a-f]{64}$ ]]; then
            echo "could not read the digest of $REGISTRY/$path:$tag" >&2
            exit 2
        fi
        [ "$now" != "$pinned" ] || continue
        echo "${file#"$ROOT"/}: $name ($path:$tag) moved"
        echo "  pinned  $pinned"
        echo "  current $now"
        drift=1
        if [ "$MODE" = --update ]; then
            sed -i "s#^\(ARG $name=$REGISTRY/$path:$tag@\)$pinned\$#\1$now#" "$file"
        fi
    done < <(sed -n "s#^ARG \([A-Z_]*\)=$REGISTRY/\([^:@ ]*\):\([^@ ]*\)@\(sha256:[0-9a-f]\{64\}\)\$#\1 \2 \3 \4#p" "$file")
done

if [ "$drift" = 1 ] && [ "$MODE" = --check ]; then
    echo "run scripts/docker-base-digests.sh --update, check that the images build, and open a pull request"
    exit 1
fi
exit 0
