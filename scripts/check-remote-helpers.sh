#!/usr/bin/env bash
# check-remote-helpers.sh — the remote helper builds a package is about to ship
# are all there, are each the right kind of binary for their platform, and run.
#
# Spec §9.1 (docs/specs/SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md):
# "a broken cross-build cannot ship". Every build is checked for its format
# (ELF or Mach-O, and the CPU). Each one this machine can run is run, and must
# print `agentmux-remote <version> protocol <N>` for this commit:
#   - Linux x86_64 runs its own build; arm64 under qemu-aarch64 if installed;
#   - macOS arm64 runs its own build, and the Intel one under Rosetta.
# On a machine that can run none of them (Windows), only the formats are checked.
#
# Usage:
#   bash scripts/check-remote-helpers.sh [DIR]   # default: dist/remote
# Exit 0 = all four present and every run printed the expected line.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

DIR="${1:-dist/remote}"
# awk, not sed: macOS's BSD sed rejects GNU sed's one-line `{s/.../p}` block.
VERSION="$(awk '/^\[workspace\.package\]/ { inpkg = 1; next } /^\[/ { inpkg = 0 } inpkg && /^version *=/ { gsub(/^version *= *"|"[[:space:]]*$/, ""); print; exit }' Cargo.toml)"
PROTOCOL="$(sed -n 's/^pub const PROTOCOL: u32 = \([0-9]*\);/\1/p' crates/remote/src/frame.rs)"
if [ -z "$VERSION" ] || [ -z "$PROTOCOL" ]; then
    echo "check-remote-helpers: could not read the version ($VERSION) or protocol ($PROTOCOL)." >&2
    exit 1
fi
WANT="agentmux-remote $VERSION protocol $PROTOCOL"

# target | expected header (hex of the first bytes that identify format and CPU)
# ELF: 7f454c46 (magic), class 02 (64-bit) ... e_machine at offset 18: 3e00 x86-64, b700 aarch64.
# Mach-O 64: cffaedfe, then cputype 07000001 (x86_64) or 0c000001 (arm64).
check_format() {
    local t="$1" f="$2" head machine
    head="$(od -An -tx1 -N20 "$f" | tr -d ' \n')"
    case "$t" in
        x86_64-unknown-linux-musl)  [ "${head:0:8}" = "7f454c46" ] && machine="${head:36:4}" && [ "$machine" = "3e00" ] ;;
        aarch64-unknown-linux-musl) [ "${head:0:8}" = "7f454c46" ] && machine="${head:36:4}" && [ "$machine" = "b700" ] ;;
        x86_64-apple-darwin)        [ "${head:0:8}" = "cffaedfe" ] && [ "${head:8:8}" = "07000001" ] ;;
        aarch64-apple-darwin)       [ "${head:0:8}" = "cffaedfe" ] && [ "${head:8:8}" = "0c000001" ] ;;
        *) return 1 ;;
    esac
}

# How to run target $1 here, if at all (prints a command prefix, or nothing).
runner_for() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os/$arch/$1" in
        Linux/x86_64/x86_64-unknown-linux-musl) echo "exec" ;;
        Linux/aarch64/aarch64-unknown-linux-musl) echo "exec" ;;
        Linux/x86_64/aarch64-unknown-linux-musl)
            if command -v qemu-aarch64-static >/dev/null 2>&1; then echo "qemu-aarch64-static";
            elif command -v qemu-aarch64 >/dev/null 2>&1; then echo "qemu-aarch64"; fi ;;
        Darwin/arm64/aarch64-apple-darwin) echo "exec" ;;
        Darwin/arm64/x86_64-apple-darwin)
            if arch -x86_64 /usr/bin/true >/dev/null 2>&1; then echo "arch -x86_64"; fi ;;
        Darwin/x86_64/x86_64-apple-darwin) echo "exec" ;;
    esac
}

fail=0
ran=0
for t in x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin; do
    f="$DIR/$t/agentmux-remote"
    if [ ! -f "$f" ]; then
        echo "check-remote-helpers: missing $f" >&2
        fail=1
        continue
    fi
    if ! check_format "$t" "$f"; then
        echo "check-remote-helpers: $f is not a $t binary" >&2
        fail=1
        continue
    fi
    r="$(runner_for "$t")"
    if [ -z "$r" ]; then
        echo "  $t: format ok (can't run it on this machine)"
        continue
    fi
    if [ "$r" = "exec" ]; then got="$("$f" version 2>&1 || true)"; else got="$($r "$f" version 2>&1 || true)"; fi
    if [ "$got" = "$WANT" ]; then
        echo "  $t: runs, \"$got\""
        ran=$((ran + 1))
    else
        echo "check-remote-helpers: $t printed \"$got\", expected \"$WANT\"" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "✓ check-remote-helpers: all four builds present and well-formed; $ran ran and printed \"$WANT\""
