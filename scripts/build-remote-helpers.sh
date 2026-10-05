#!/usr/bin/env bash
# build-remote-helpers.sh — build agentmux-remote for every SSH host platform.
#
# The helper runs on the remote host, not on this machine, and any AgentMux can
# connect to any host, so every desktop package carries all four builds
# (docs/specs/SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md §9.1).
# cargo-zigbuild cross-builds all four from any of our build machines (Windows,
# Linux, macOS): static musl for Linux, and both macOS architectures.
#
# Usage:
#   bash scripts/build-remote-helpers.sh [OUT_DIR]     # default: dist/remote
#
# Writes OUT_DIR/<target>/agentmux-remote for each target.
#
# REQUIRE_REMOTE_HELPERS=1 (release builds): any missing toolchain or failed
# build is an error. Otherwise (a local `task package`) a missing toolchain is a
# warning: the package is built without helpers, and AgentMux then can't
# install one on an SSH host (file browsing and durable sessions there).
#
# Needs: zig (`pip install ziglang`) and cargo-zigbuild
# (`cargo install cargo-zigbuild --locked`).

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

OUT_DIR="${1:-dist/remote}"
REQUIRE="${REQUIRE_REMOTE_HELPERS:-0}"
TARGETS=(
    x86_64-unknown-linux-musl
    aarch64-unknown-linux-musl
    x86_64-apple-darwin
    aarch64-apple-darwin
)

# Clear any earlier output first, so a package can never pick up a stale build of
# another version, including when this run skips for a missing toolchain.
rm -rf "$OUT_DIR"

missing() {
    if [ "$REQUIRE" = "1" ]; then
        echo "build-remote-helpers: $1" >&2
        exit 1
    fi
    echo "⚠ build-remote-helpers: $1" >&2
    echo "  Packaging continues without the remote helper: this build can't install it on an SSH host." >&2
    echo "  To include it: pip install ziglang && cargo install cargo-zigbuild --locked" >&2
    exit 0
}

command -v cargo-zigbuild >/dev/null 2>&1 || missing "cargo-zigbuild is not installed"
command -v rustup >/dev/null 2>&1 || missing "rustup is not installed"
# cargo-zigbuild finds zig on PATH or as the `ziglang` Python package.
if ! command -v zig >/dev/null 2>&1 \
    && ! python3 -m ziglang version >/dev/null 2>&1 \
    && ! python -m ziglang version >/dev/null 2>&1; then
    missing "zig is not installed"
fi

installed_targets="$(rustup target list --installed)"
for t in "${TARGETS[@]}"; do
    if ! printf '%s\n' "$installed_targets" | grep -qx "$t"; then
        rustup target add "$t" >/dev/null
    fi
done

for t in "${TARGETS[@]}"; do
    cargo zigbuild --release --quiet -p agentmux-remote --target "$t"
    mkdir -p "$OUT_DIR/$t"
    cp "target/$t/release/agentmux-remote" "$OUT_DIR/$t/agentmux-remote"
    chmod 755 "$OUT_DIR/$t/agentmux-remote"
done

echo "✓ Remote helpers in $OUT_DIR:"
for t in "${TARGETS[@]}"; do
    printf '  %-28s %s bytes\n' "$t" "$(wc -c < "$OUT_DIR/$t/agentmux-remote" | tr -d ' ')"
done

# Every build present, the right format, and each one this machine can run
# prints this commit's version and protocol.
bash scripts/check-remote-helpers.sh "$OUT_DIR"
