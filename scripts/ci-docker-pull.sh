#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Pull the images a CI job runs, without Docker Hub's anonymous limit.
#
#   scripts/ci-docker-pull.sh ubuntu:24.04 fedora:42 opensuse/tumbleweed
#
# Docker Hub limits anonymous pulls per address, and the hosted runners share addresses, so a plain
# `docker run ubuntu:24.04` can fail a job for no fault of the change ("toomanyrequests"). For each image
# this tries, in turn, a mirror of the same image that needs no sign-in, then the next, then Docker Hub
# itself last, three tries each with a growing wait (a mirror limits anonymous pulls too: the release
# package job was refused by Amazon's once). When one answers, the image is tagged with the name it was
# asked by, so the `docker run` / `docker build --build-arg BASE=...` that follows finds it on the
# machine and pulls nothing. No credential is used or needed. A tag is a moving target as it always was:
# this changes where the bytes come from, not which image the tag names.
#
# Env: CI_DOCKER_PULL_TRIES (3), CI_DOCKER_PULL_WAIT (seconds before the second try, doubled each time; 15).
set -euo pipefail

TRIES=${CI_DOCKER_PULL_TRIES:-3}
WAIT=${CI_DOCKER_PULL_WAIT:-15}

# The places an image can be pulled from, most reliable first, one per line. A one-word name is an
# official image (`library/<name>`); an organisation's image keeps its path, and openSUSE's own registry
# serves its images itself.
sources() {
    local ref=$1 path=$1
    case "$ref" in
        */*) ;;
        *) path="library/$ref" ;;
    esac
    case "$ref" in
        opensuse/*) echo "registry.opensuse.org/$ref" ;;
    esac
    case "$ref" in
        */*) ;;
        *) echo "public.ecr.aws/docker/$path" ;;
    esac
    echo "mirror.gcr.io/$path"
    echo "docker.io/$path"
}

pull_one() {
    local ref=$1 source attempt wait
    while read -r source; do
        [ -n "$source" ] || continue
        wait=$WAIT
        for ((attempt = 1; attempt <= TRIES; attempt++)); do
            if docker pull --quiet "$source" >/dev/null 2>&1; then
                [ "$source" = "$ref" ] || docker tag "$source" "$ref"
                echo "ci-docker-pull: $ref from $source"
                return 0
            fi
            echo "::warning::ci-docker-pull: $source did not answer for $ref (try $attempt of $TRIES)" >&2
            if [ "$attempt" -lt "$TRIES" ]; then sleep "$wait"; wait=$((wait * 2)); fi
        done
    done < <(sources "$ref")
    echo "::error::ci-docker-pull: no source would give $ref" >&2
    return 1
}

[ "$#" -gt 0 ] || { echo "usage: ci-docker-pull.sh <image>..." >&2; exit 2; }
status=0
for ref in "$@"; do
    pull_one "$ref" || status=1
done
exit "$status"
