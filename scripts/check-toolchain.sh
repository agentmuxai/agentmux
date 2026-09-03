#!/usr/bin/env bash
# Checks that the tools task dev/task package actually need are present,
# before npm install runs and fails with a confusing error instead. See
# docs/reports/REPORT_FRESH_PC_ONBOARDING_AUDIT_2026_09_02.md and tracking
# issue #2940 — nothing in this repo checked for Rust/CMake/Ninja/git
# presence before this, so a missing one surfaced as a raw cargo/cef-dll-sys
# build error deep into `task dev`, not a clear message up front.
#
# Deliberately does NOT check for Task itself — reaching this script at all
# already requires Task to be installed and working. That check lives in
# scripts/bootstrap.sh / scripts/bootstrap.ps1 instead, which run before
# Task exists (Codex review finding on PR #2941/#2940: a Task-based check
# for Task is circular on exactly the fresh-PC scenario this exists for).
set -uo pipefail

case "$(uname -s)" in
  Darwin) OS_NAME=macos ;;
  Linux) OS_NAME=linux ;;
  MINGW*|MSYS*|CYGWIN*) OS_NAME=windows ;;
  *) OS_NAME=unknown ;;
esac

FAIL=0

check() {
  local name="$1" cmd="$2" instructions="$3"
  if command -v "$cmd" >/dev/null 2>&1; then
    local version
    version="$("$cmd" --version 2>/dev/null | head -1)"
    echo "  OK   $name — $version"
  else
    echo "  MISSING  $name"
    echo "           $instructions"
    FAIL=1
  fi
}

echo "Checking required toolchain (platform: $OS_NAME)..."
echo

check "git" git "https://git-scm.com/downloads"

case "$OS_NAME" in
  macos)
    check "rust (cargo)" cargo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    check "cmake" cmake "brew install cmake"
    check "ninja" ninja "brew install ninja"
    ;;
  linux)
    check "rust (cargo)" cargo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    check "cmake" cmake "sudo apt install cmake ninja-build build-essential curl wget file libssl-dev git zip libwayland-dev libxkbcommon-dev libgtk-3-dev libglib2.0-dev libpango1.0-dev libcairo2-dev libgdk-pixbuf2.0-dev libatk1.0-dev"
    check "ninja" ninja "sudo apt install cmake ninja-build build-essential curl wget file libssl-dev git zip libwayland-dev libxkbcommon-dev libgtk-3-dev libglib2.0-dev libpango1.0-dev libcairo2-dev libgdk-pixbuf2.0-dev libatk1.0-dev"
    ;;
  windows)
    check "rust (cargo)" cargo "https://rustup.rs/ (rustup-init.exe), then install Visual Studio Build Tools with the 'Desktop development with C++' workload"
    check "cmake" cmake "Ships with Visual Studio Build Tools — install the 'Desktop development with C++' workload from https://visualstudio.microsoft.com/visual-cpp-build-tools/"
    check "ninja" ninja "Ships with Visual Studio but is not always on PATH — see CLAUDE.md's Build Prerequisites section for the exact copy command"
    ;;
  *)
    echo "  Unrecognized platform ($OS_NAME) — skipping cmake/ninja/rust checks. See BUILD.md."
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
  echo "One or more required tools are missing or the wrong version. Install them and re-run 'task init'."
  exit 1
fi

echo "All required tools present."
