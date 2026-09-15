#!/usr/bin/env bash
# Print the absolute path of the directory that contains a
# `Chromium Embedded Framework.framework` suitable for AgentMux to
# load on macOS.
#
# Why this script exists
# ----------------------
# cef-rs's LibraryLoader resolves the framework relative to the host
# executable via `../Frameworks/Chromium Embedded Framework.framework/
# Chromium Embedded Framework` and `.canonicalize().unwrap()`s — so if
# the framework isn't on disk in the expected layout, the host panics
# the instant it starts. `task bundle:darwin` calls this script to
# locate a usable framework and then `ditto`s it into `dist/Frameworks/`.
#
# Resolution order
# ----------------
#   1. $AGENTMUX_CEF_RUNTIME_DIR_DARWIN — explicit override
#      (set when bundling a custom or patched framework).
#   2. $HOME/cef-build/darwin/<arch>/                       — the
#      cef-build standard layout (analogous to Linux's
#      $HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64/).
#   3. cef-dll-sys cargo cache — first match of
#      <repo>/target/{release,debug}/build/cef-dll-sys-*/out/cef_macos_<arch>/
#      Release first so dev/package builds don't pick up a stale debug
#      framework.
#
# Each candidate is validated by checking for
# `<candidate>/Chromium Embedded Framework.framework/Chromium Embedded Framework`.
#
# Output
# ------
# stdout: absolute path of the chosen directory (the one that contains
#         the .framework — not the .framework itself).
# stderr: progress info + actionable errors.
# exit 0 on success, 1 if no candidate had the framework.
#
# Used by Taskfile.yml::bundle:darwin. See
# docs/specs/SPEC_MACOS_CEF_FRAMEWORK_BUNDLING_2026_05_28.md.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Codex P1 on PR #3231: candidates below were validated by file PRESENCE only
# — never by version. A pre-existing local ~/cef-build/darwin tree from before
# a CEF milestone bump would be silently accepted, producing a runtime/
# binding-version mismatch bundle. Same technique
# scripts/resolve-cef-runtime.sh's Linux twin uses (see its own comment for
# the cross-platform verification this was based on): `grep -a` for CEF's
# plain-text version string, which is embedded as readable bytes in the
# framework binary the same way it is in libcef.so/libcef.dll. Warns rather
# than hard-fails — matches this script's own existing warn-not-fail posture
# (no hard-fail path exists here at all outside the explicit-override case);
# CI's scripts/verify-cef-version.sh remains the hard gate for anything shipped.
expected_cef_major() {
    local metadata pkg_id full target
    case "$ARCH" in
        aarch64) target="aarch64-apple-darwin" ;;
        x86_64)  target="x86_64-apple-darwin" ;;
        *) return 1 ;;
    esac
    metadata="$(cargo metadata --manifest-path "$REPO_ROOT/Cargo.toml" --filter-platform "$target" --format-version 1 2>/dev/null)" || return 1
    pkg_id="$(printf '%s' "$metadata" | jq -r '
        .resolve.nodes[] | select(.id | startswith("path+file://") and contains("/agentmux-cef#"))
        | .deps[] | select(.name == "cef") | .pkg
    ' 2>/dev/null)"
    [ -n "$pkg_id" ] || return 1
    full="$(printf '%s' "$metadata" | jq -r --arg pkg "$pkg_id" '.packages[] | select(.id == $pkg) | .version' 2>/dev/null)"
    printf '%s' "$full" | sed -n 's/^\([0-9][0-9]*\).*/\1/p'
}

check_version() {
    local dir="$1" fw="$dir/Chromium Embedded Framework.framework/Chromium Embedded Framework"
    local expected_major actual_major
    expected_major="$(expected_cef_major)" || return 0
    [ -n "$expected_major" ] || return 0
    actual_major="$(grep -a -o "${expected_major}\.[0-9]*\.[0-9]*" "$fw" 2>/dev/null | head -1 | cut -d. -f1)"
    if [ -z "$actual_major" ]; then
        echo "WARNING: could not confirm the framework at $dir is CEF ${expected_major}.x" >&2
        echo "         (expected the version string embedded in the binary; found none matching)." >&2
        echo "         If this is a stale pre-upgrade build, task bundle:darwin will ship a" >&2
        echo "         runtime/binding mismatch. Rebuild per" >&2
        echo "         docs/cef-build/build-patched-framework-macos.md if in doubt." >&2
    fi
}

# cef-dll-sys's build script writes its output to `cef_macos_aarch64`
# (Rust's `target_arch` convention), NOT `cef_macos_arm64` (Apple's
# ecosystem convention). The script HAS to match that directory layout
# or the cargo-cache fallback silently misses every candidate. The fact
# that `agentmux-cef/src/sidecar.rs:386` separately maps `aarch64 →
# arm64` is for the agentmux-srv binary FILENAME, not directory naming
# — different concern. Don't "fix" this to arm64; verified empirically:
#   $ ls target/release/build/cef-dll-sys-*/out/ | grep cef_macos
#   cef_macos_aarch64
case "$(uname -m)" in
    arm64)   ARCH="aarch64" ;;
    x86_64)  ARCH="x86_64"  ;;
    *) echo "❌ unsupported macOS arch: $(uname -m)" >&2; exit 1 ;;
esac

# `validate_dir <dir> [kind]` — print to stdout and exit 0 if `<dir>` contains
# a usable framework. Returns 1 (no output) otherwise. `kind=cargo-cache`
# skips the version check below — that candidate is always exactly whatever
# Cargo.lock resolved, so checking it is redundant (unlike an
# operator/dev-built tree, which can silently go stale across a CEF bump).
validate_dir() {
    local dir="$1" kind="${2:-cef-build}"
    if [ -f "$dir/Chromium Embedded Framework.framework/Chromium Embedded Framework" ]; then
        echo "[resolve-cef-runtime-darwin] using: $dir" >&2
        [ "$kind" = "cargo-cache" ] || check_version "$dir"
        printf '%s\n' "$dir"
        exit 0
    fi
    return 1
}

# 1. Explicit override
if [ -n "${AGENTMUX_CEF_RUNTIME_DIR_DARWIN:-}" ]; then
    validate_dir "$AGENTMUX_CEF_RUNTIME_DIR_DARWIN" || true
    echo "❌ AGENTMUX_CEF_RUNTIME_DIR_DARWIN is set but the framework is missing:" >&2
    echo "   expected: $AGENTMUX_CEF_RUNTIME_DIR_DARWIN/Chromium Embedded Framework.framework/Chromium Embedded Framework" >&2
    exit 1
fi

# 2. Standard cef-build layout
validate_dir "$HOME/cef-build/darwin/$ARCH" || true

# 3. cef-dll-sys cargo cache — release first (production / `task dev` builds
# pass `--release` to cargo), then debug as a fallback. The shebang pins us
# to bash, so an unmatched glob iterates over the literal pattern; the
# `[ -d "$cand" ]` guard handles that without needing nullglob.
for profile in release debug; do
    for cand in "$REPO_ROOT/target/$profile/build/cef-dll-sys-"*"/out/cef_macos_$ARCH"; do
        [ -d "$cand" ] || continue
        validate_dir "$cand" cargo-cache || true
    done
done

echo "❌ No Chromium Embedded Framework.framework found in any candidate location:" >&2
echo "   1. \$AGENTMUX_CEF_RUNTIME_DIR_DARWIN (unset)" >&2
echo "   2. \$HOME/cef-build/darwin/$ARCH" >&2
echo "   3. cef-dll-sys cargo cache (target/{release,debug}/build/cef-dll-sys-*/out/cef_macos_$ARCH/)" >&2
echo "" >&2
echo "   Build the cef-dll-sys download step first (cargo build -p agentmux-cef" >&2
echo "   triggers it), or set AGENTMUX_CEF_RUNTIME_DIR_DARWIN to a directory" >&2
echo "   containing the framework." >&2
exit 1
