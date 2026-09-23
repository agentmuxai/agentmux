#!/usr/bin/env bash
# check-name-resolver-callers.sh — CI grep gate for identity M3.
#
# SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §2 rule 1 / §9.3:
#
#   "No internal code path ever converts a name to an identity. A function
#    that takes a name and returns an identity exists at exactly one layer,
#    and no internal caller may use it."
#
# That function is `resolve_name_to_uid` (backend/name_resolution.rs). It may
# be referenced ONLY by:
#   - its own module (definition + tests)
#   - server/name_resolution.rs — the HTTP endpoint the MCP boundary calls
#   - this script, the spec, and the docs that cite it
#
# Any other reference FAILS the build. A caller that wants an identity from
# a name has to get it from the boundary (the MCP tool argument, resolved by
# the endpoint) or carry one it already has — never derive it internally.
set -euo pipefail
cd "$(dirname "$0")/.."

ALLOWED=(
    "agentmux-srv/src/backend/name_resolution.rs"
    "agentmux-srv/src/server/name_resolution.rs"
)

violations=$(grep -rn --include='*.rs' 'resolve_name_to_uid' agentmux-srv agentmux-mcp agentmux-common agentmux-launcher agentmux-bashwrap 2>/dev/null \
    | grep -v -F "${ALLOWED[0]}" \
    | grep -v -F "${ALLOWED[1]}" \
    || true)

if [[ -n "$violations" ]]; then
    echo "check-name-resolver-callers: resolve_name_to_uid is referenced outside the human-boundary layer:" >&2
    echo "$violations" >&2
    echo "" >&2
    echo "Internal code must carry an identity it already has, or receive one from the MCP boundary." >&2
    echo "See SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §2 rule 1 and §9.3." >&2
    exit 1
fi
echo "check-name-resolver-callers: ok"
