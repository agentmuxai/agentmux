#!/bin/sh
# Checks that the tools task dev/task package actually need are present AND
# meet the documented minimum versions, before npm install runs and fails
# with a confusing error instead. See
# docs/reports/REPORT_FRESH_PC_ONBOARDING_AUDIT_2026_09_02.md and tracking
# issue #2940 — nothing in this repo checked for Rust/CMake/Ninja/git
# presence before this, so a missing (or too-old) one surfaced as a raw
# cargo/cef-dll-sys build error deep into `task dev`, not a clear message up
# front (Codex review finding on PR #2943: an earlier version of this script
# checked presence only, so an obsolete cargo/cmake/ninja still reported OK
# and the same later build failure this exists to prevent still happened).
#
# Deliberately does NOT check for Task itself — reaching this script at all
# already requires Task to be installed and working. That check lives in
# scripts/bootstrap.sh / scripts/bootstrap.ps1 instead, which run before
# Task exists (Codex/reagent review finding on PR #2941/#2943: a Task-based
# check for Task is circular on exactly the fresh-PC scenario this exists
# for). For the same reason this script itself is invoked via `sh`, not
# `bash` — bootstrap.ps1 is responsible for getting a shell onto PATH on
# Windows in the first place.
set -u

case "$(uname -s)" in
  Darwin) OS_NAME=macos ;;
  Linux) OS_NAME=linux ;;
  MINGW*|MSYS*|CYGWIN*) OS_NAME=windows ;;
  *) OS_NAME=unknown ;;
esac

FAIL=0

# $1 >= $2 for dotted version strings (e.g. "1.93.0" "1.77"). Pure POSIX —
# no bc/awk dependency, so it works identically in dash, ash/busybox sh, and
# bash's sh-compat mode.
version_ge() {
  i=1
  while true; do
    p1="$(echo "$1" | cut -d. -f"$i")"
    p2="$(echo "$2" | cut -d. -f"$i")"
    [ -z "$p1" ] && [ -z "$p2" ] && return 0
    p1="${p1:-0}"
    p2="${p2:-0}"
    if [ "$p1" -gt "$p2" ] 2>/dev/null; then return 0; fi
    if [ "$p1" -lt "$p2" ] 2>/dev/null; then return 1; fi
    i=$((i + 1))
  done
}

extract_version() {
  echo "$1" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+|[0-9]+\.[0-9]+' | head -1
}

# check <display name> <command> <min version, or empty to skip the check> <install instructions>
check() {
  local name="$1" cmd="$2" min="$3" instructions="$4"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "  MISSING  $name"
    echo "           $instructions"
    FAIL=1
    return
  fi

  local raw
  raw="$("$cmd" --version 2>/dev/null | head -1)"

  if [ -z "$min" ]; then
    echo "  OK   $name — $raw"
    return
  fi

  local found
  found="$(extract_version "$raw")"
  if [ -z "$found" ]; then
    echo "  OK   $name — $raw (could not parse version to check against >=$min; assuming fine)"
    return
  fi

  if version_ge "$found" "$min"; then
    echo "  OK   $name — $raw"
  else
    echo "  TOO OLD  $name — found $found, need >=$min"
    echo "           $instructions"
    FAIL=1
  fi
}

echo "Checking required toolchain (platform: $OS_NAME)..."
echo

check "git" git "" "https://git-scm.com/downloads"

case "$OS_NAME" in
  macos)
    check "rust (cargo)" cargo "1.77" "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh, or: rustup update"
    check "cmake" cmake "3.20" "brew install cmake  (or: brew upgrade cmake)"
    check "ninja" ninja "1.10" "brew install ninja  (or: brew upgrade ninja)"
    ;;
  linux)
    check "rust (cargo)" cargo "1.77" "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh, or: rustup update"
    check "cmake" cmake "3.20" "sudo apt install cmake ninja-build build-essential curl wget file libssl-dev git zip libwayland-dev libxkbcommon-dev libgtk-3-dev libglib2.0-dev libpango1.0-dev libcairo2-dev libgdk-pixbuf2.0-dev libatk1.0-dev"
    check "ninja" ninja "1.10" "sudo apt install cmake ninja-build build-essential curl wget file libssl-dev git zip libwayland-dev libxkbcommon-dev libgtk-3-dev libglib2.0-dev libpango1.0-dev libcairo2-dev libgdk-pixbuf2.0-dev libatk1.0-dev"
    ;;
  windows)
    check "rust (cargo)" cargo "1.77" "https://rustup.rs/ (rustup-init.exe), then install Visual Studio Build Tools with the 'Desktop development with C++' workload, or: rustup update"
    check "cmake" cmake "3.20" "Ships with Visual Studio Build Tools — install the 'Desktop development with C++' workload from https://visualstudio.microsoft.com/visual-cpp-build-tools/"
    check "ninja" ninja "1.10" "Ships with Visual Studio but is not always on PATH — see CLAUDE.md's Build Prerequisites section for the exact copy command"
    ;;
  *)
    echo "  Unrecognized platform ($OS_NAME) — skipping cmake/ninja/rust version checks. See BUILD.md."
    ;;
esac

echo

if command -v node >/dev/null 2>&1; then
  NODE_VERSION="$(node --version 2>/dev/null)"
  NODE_MAJOR="${NODE_VERSION#v}"
  NODE_MAJOR="${NODE_MAJOR%%.*}"
  if [ -n "$NODE_MAJOR" ] && [ "$NODE_MAJOR" -ge 24 ] 2>/dev/null; then
    echo "  OK   node.js — $NODE_VERSION"
  else
    echo "  WRONG VERSION  node.js — found $NODE_VERSION, need >=24 (see .nvmrc)"
    echo "                 https://nodejs.org/ or nvm install 24"
    FAIL=1
  fi
else
  echo "  MISSING  node.js"
  echo "           https://nodejs.org/ or nvm install 24 — need >=24, see .nvmrc"
  FAIL=1
fi

echo

if [ "$FAIL" -ne 0 ]; then
  echo "One or more required tools are missing or below the minimum version. Install/upgrade them and re-run 'task init'."
  exit 1
fi

echo "All required tools present and up to date."
