#!/usr/bin/env bash
# check-bundled-tools.sh — CI grep gate: every release packager ships every tool
# that agents run by bare name.
#
# THE RULE: each binary that agent_config.rs writes into an agent's config as a
# bare `"command": "agentmux-<name> ..."` (the Bash PreToolUse/PreCompact hooks,
# the agentmux MCP server) must be copied into the bundled tools/bin directory
# by every release packager. agentmux-srv puts <exe_dir>/tools/bin on the
# agent's PATH, and that is the only place these names resolve.
#
# WHY: macOS and Linux packages shipped agentmux-mcp but never
# agentmux-bashwrap, from their first release until this gate. A missing hook
# binary is not an error anyone sees: Claude Code treats the hook's
# command-not-found as a non-blocking hook error and runs every Bash call
# unwrapped, so streamed Bash output, the idle-timeout guard and background-task
# pid/exit reporting were all silently off there. Windows (package-portable.sh)
# always shipped both, and `task dev` copies both on every platform, so no one
# developing the feature could see the gap. Found while root-causing Activity
# Dock rows stuck "running" for 12+ hours on a packaged macOS build.
#
# Usage:
#   bash scripts/check-bundled-tools.sh
# Exit 0 = every packager bundles every tool, exit 1 = something is missing.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

CONFIG=crates/srv/src/backend/agent_config.rs

# One release packager per platform: each is where tools/bin gets populated.
PACKAGERS=(
    scripts/package-portable.sh   # Windows portable (the installer wraps it)
    scripts/package-macos.sh      # macOS .app / DMG
    scripts/stage-linux-runtime.sh # Linux: shared by AppImage, deb, rpm, tarball
)

# The tool names come from the config writer itself, so a new bare-name tool
# added there is covered without editing this list. (A read loop, not mapfile:
# macOS still ships bash 3.2.)
TOOLS=()
while IFS= read -r t; do TOOLS+=("$t"); done < <(grep -oE '"command": "agentmux-[a-z0-9-]+' "$CONFIG" | sed 's/.*"agentmux-/agentmux-/' | sort -u)

if [ "${#TOOLS[@]}" -eq 0 ]; then
    echo "check-bundled-tools: found no \"command\": \"agentmux-...\" entries in $CONFIG." >&2
    echo "  The config format changed; update this gate's extraction rather than letting it pass vacuously." >&2
    exit 1
fi

fail=0
for p in "${PACKAGERS[@]}"; do
    if [ ! -f "$p" ]; then
        echo "check-bundled-tools: packager $p no longer exists; update PACKAGERS." >&2
        fail=1
        continue
    fi
    for t in "${TOOLS[@]}"; do
        # A cp of target/release/<tool>[.exe] whose destination is a tools/bin
        # dir, or (macOS, where each tool runs from its own helper app) a cp of
        # it anywhere plus a symlink at tools/bin/<tool>.
        if ! grep -qE "^[[:space:]]*cp .*target/release/${t}(\\.exe)?[\" ].*tools/bin" "$p" &&
           ! { grep -qE "^[[:space:]]*cp .*target/release/${t}(\\.exe)?[\" ]" "$p" &&
               grep -qE "tools/bin/${t}\"?[[:space:]]*$" "$p" && grep -qE "^[[:space:]]*ln -s .*${t}" "$p"; }; then
            echo "check-bundled-tools: $p does not copy $t into tools/bin." >&2
            fail=1
        fi
    done
done

# The remote helpers (docs/specs/SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md
# §9.1): every packager builds them and copies them into the package. A package
# without them can't install the helper on an SSH host, so file browsing and
# durable sessions there fail, and nothing else in CI would notice.
for p in "${PACKAGERS[@]}"; do
    [ -f "$p" ] || continue
    if ! grep -q 'scripts/build-remote-helpers.sh' "$p" || ! grep -qE '(tools|Resources)/remote' "$p"; then
        echo "check-bundled-tools: $p does not build the remote helpers (scripts/build-remote-helpers.sh) into tools/remote." >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo "" >&2
    echo "Agents run these tools by bare name (see $CONFIG). A packager that omits one" >&2
    echo "ships a build where that hook or MCP server silently never runs." >&2
    exit 1
fi

echo "check-bundled-tools: ${#PACKAGERS[@]} packagers each bundle: ${TOOLS[*]}, and the remote helpers"
