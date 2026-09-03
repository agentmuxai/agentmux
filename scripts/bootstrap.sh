#!/usr/bin/env sh
# Entry point for a brand-new macOS/Linux machine with nothing installed yet
# — deliberately POSIX sh, not bash, and deliberately outside Task, because
# reaching `task init` at all already requires Task to be installed and
# working (Codex review finding on the fresh-PC onboarding audit, #2940:
# `task init` cannot be what detects/installs Task itself, that's circular
# on exactly the scenario this exists for).
#
# This script only gets you as far as `task` being on PATH. Everything
# after that — Rust/Node/CMake/Ninja/git — is `task init`'s job
# (scripts/check-toolchain.sh), run once Task itself exists.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/agentmuxai/agentmux/main/scripts/bootstrap.sh | sh
#   # or, after cloning:
#   sh scripts/bootstrap.sh
set -eu

if command -v task >/dev/null 2>&1; then
  echo "Task is already installed: $(task --version)"
  echo "Next: task init"
  exit 0
fi

case "$(uname -s)" in
  Darwin)
    echo "Task not found. Installing via Homebrew..."
    if ! command -v brew >/dev/null 2>&1; then
      echo "Homebrew is not installed either. Install it first: https://brew.sh/"
      echo "Then re-run this script, or install Task directly: https://taskfile.dev/installation/"
      exit 1
    fi
    brew install go-task/tap/go-task
    ;;
  Linux)
    echo "Task not found. Installing via snap..."
    if ! command -v snap >/dev/null 2>&1; then
      echo "snap is not available on this system. Install Task manually: https://taskfile.dev/installation/"
      exit 1
    fi
    sudo snap install task --classic
    ;;
  *)
    echo "Unrecognized platform ($(uname -s)). Install Task manually: https://taskfile.dev/installation/"
    exit 1
    ;;
esac

echo
echo "Task installed: $(task --version)"
echo "Next: clone the repo if you haven't, cd into it, then run: task init"
