#!/usr/bin/env bash
# Smoke test for the container agent developer image: does it still build AgentMux?
#
#   scripts/smoke-developer-image.sh [--image IMAGE] [--ref REF] [--stage check|dev] [--out DIR] [--repo URL]
#
#   --image  the image to test (default ghcr.io/agentmuxai/agent-base:developer)
#   --ref    a branch, tag or commit of the repo to build inside the container (default main)
#   --stage  check: the pinned Rust matches the repo, then `cargo check --tests -p agentmux-srv` (about 7 min cold)
#            dev:   check, then `agent-dev` (Xvfb + `task dev`) until the window is up and stays up, with a screenshot
#            (about 16 min cold, 12 GB of build output). Default check.
#   --out    where the screenshot goes (default ./smoke-developer-image-out)
#   --repo   the repository to clone (default https://github.com/agentmuxai/agentmux.git, public)
#
# One container, started with `docker run --rm`, with named volumes for cargo's home and the build output (the
# directories the image's ai.agentmux.cache-dirs label names, so they are writable by `agent` from the start).
# The volumes are removed at the end, pass or fail. Exit 0 only when every step passed.
# Spec: docs/specs/SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md section 6.
set -euo pipefail

IMAGE="ghcr.io/agentmuxai/agent-base:developer"
REF="main"
STAGE="check"
OUT="./smoke-developer-image-out"
REPO="https://github.com/agentmuxai/agentmux.git"
DEV_TIMEOUT_S="${SMOKE_DEV_TIMEOUT_S:-2700}"
HOLD_S="${SMOKE_DEV_HOLD_S:-30}"

usage() { sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; }
while [ $# -gt 0 ]; do
    case "$1" in
        --image) IMAGE="${2:?--image needs a value}"; shift 2 ;;
        --ref) REF="${2:?--ref needs a value}"; shift 2 ;;
        --stage) STAGE="${2:?--stage needs a value}"; shift 2 ;;
        --out) OUT="${2:?--out needs a value}"; shift 2 ;;
        --repo) REPO="${2:?--repo needs a value}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "smoke-developer-image: unknown argument $1" >&2; usage >&2; exit 2 ;;
    esac
done
case "$STAGE" in check|dev) ;; *) echo "smoke-developer-image: --stage is check or dev, not '$STAGE'" >&2; exit 2 ;; esac
command -v docker >/dev/null || { echo "smoke-developer-image: docker is not on PATH" >&2; exit 2; }

RUN_ID="smoke-dev-$$-$(date +%s)"
VOLUMES=("$RUN_ID-cargo" "$RUN_ID-target")
cleanup() { docker volume rm -f "${VOLUMES[@]}" >/dev/null 2>&1 || true; }
trap cleanup EXIT
mkdir -p "$OUT"
chmod 777 "$OUT"
OUT_ABS="$(cd "$OUT" && pwd)"

# Runs inside the container as `agent`.
read -r -d '' INSIDE <<'EOS' || true
set -euo pipefail
step() { printf '\n== %s\n' "$1"; STEP_START=$(date +%s); }
done_step() { printf '   ok (%ss)\n' "$(( $(date +%s) - STEP_START ))"; }

step "tools the image promises"
for t in rustc cargo cargo-clippy rustfmt task cargo-nextest git gh python3 node npm Xvfb scrot agent-dev aws cdk; do
    command -v "$t" >/dev/null || { echo "missing from the image: $t" >&2; exit 1; }
done
rustc --version; task --version; cargo nextest --version | head -n 1
done_step

step "clone $REPO at $REF"
mkdir -p "$HOME/work/agentmux" && cd "$HOME/work/agentmux"
git init -q . && git remote add origin "$REPO"
git fetch -q --depth 1 origin "$REF" && git checkout -q FETCH_HEAD
git log --oneline -1
done_step

step "the image's Rust is the version the repo pins"
want="$(sed -n 's/^channel *= *"\([0-9][0-9.]*\)".*/\1/p' rust-toolchain.toml | head -n 1)"
[ -n "$want" ] || { echo "no channel in rust-toolchain.toml" >&2; exit 1; }
# Asked outside the checkout, so rustup cannot install the pinned version and hide a stale image.
have="$(cd /tmp && rustc --version | awk '{print $2}')"
[ "$have" = "$want" ] || { echo "the image has Rust $have, the repo pins $want: rebuild the image" >&2; exit 1; }
echo "Rust $have"
done_step

step "cargo check --tests -p agentmux-srv"
cargo check --tests -p agentmux-srv 2>&1 | tail -n 3
done_step

if [ "$STAGE" = dev ]; then
    step "agent-dev: task dev to a running window"
    agent-dev "$PWD" >/dev/null
    waited=0
    while :; do
        state="$(agent-dev status)"
        [ "$state" = up ] && break
        if [ "$state" != starting ] || [ "$waited" -ge "$DEV_TIMEOUT_S" ]; then
            echo "agent-dev: '$state' after ${waited}s; log tail:" >&2
            tail -n 40 "$HOME/.agent-dev/task-dev.log" >&2 || true
            exit 1
        fi
        sleep 10; waited=$((waited + 10))
    done
    echo "window up after about ${waited}s; holding ${HOLD_S}s"
    sleep "$HOLD_S"
    [ "$(agent-dev status)" = up ] || { echo "the window did not stay up" >&2; tail -n 40 "$HOME/.agent-dev/task-dev.log" >&2; exit 1; }
    agent-dev screenshot /out/agent-dev.png
    agent-dev stop >/dev/null
    done_step
fi
printf '\nSMOKE PASS (%s, %s, stage %s)\n' "$REF" "$(cd /tmp && rustc --version)" "$STAGE"
EOS

docker run --rm --name "$RUN_ID" \
    -v "${VOLUMES[0]}:/home/agent/.cargo" \
    -v "${VOLUMES[1]}:/home/agent/target" \
    -v "$OUT_ABS:/out" \
    -e "REPO=$REPO" -e "REF=$REF" -e "STAGE=$STAGE" \
    -e "DEV_TIMEOUT_S=$DEV_TIMEOUT_S" -e "HOLD_S=$HOLD_S" \
    "$IMAGE" bash -c "$INSIDE"
