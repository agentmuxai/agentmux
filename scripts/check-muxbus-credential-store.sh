#!/usr/bin/env bash
# check-muxbus-credential-store.sh — CI grep gate for muxbus credential reads.
#
# THE RULE: muxbus credentials — the account token (`muxbus_load`/`muxbus_save`/
# `muxbus_clear`, `load_valid_token`) and the per-agent M2M credentials
# (`ensure_agent_credential`, `relay_token`) — live in `AppState::id_store`.
# That is where `CloudSubscriber::init_global` and every `muxbus.login` /
# `muxbus.status` / `muxbus.disconnect` handler write them. Reading them from
# the per-channel `state.wstore` finds NOTHING whenever the shared root
# resolves, which is the normal case.
#
# THE REGRESSION (reagent P0 on PR #3023): the delivery tier-4 cloud relay
# called `relay_token(source, &state.wstore, ...)`. Every lookup missed,
# `relay_token` always returned `None`, and tier 4 silently never fired even
# for a properly logged-in user — the entire feature defeated by one field
# name, with no error anywhere. It logged "not logged in to muxbus" and
# returned, which reads like correct behaviour.
#
# Why a grep gate and not a unit test: `server::tests::test_state()` backs
# `wstore`, `id_store` AND `identity_store` with a single in-memory Store, so a
# runtime test cannot distinguish them — seeding a credential into one makes it
# visible through all three, and the test passes whichever field the code reads.
# The bug is a static wiring mistake and only a static check catches it.
#
# Note this rule deliberately says `id_store`, not `identity_store`, even
# though AppState's own doc comment steers new muxbus call sites toward the
# latter: the credentials are WRITTEN to `id_store`, and reading from a store
# the writer doesn't use would reintroduce this same bug whenever
# `isolated_auth_enabled()` redirects one but not the other. If the writers
# ever migrate, migrate the readers in the same change and update this gate.
#
# Usage:
#   bash scripts/check-muxbus-credential-store.sh
# Exit 0 = clean, exit 1 = a muxbus credential call was handed `state.wstore`.

set -euo pipefail

cd "$(dirname "$0")/.."

# Only `state.wstore` is matched, not a bare `wstore` identifier: several
# functions inside `muxbus/` name their own store parameter `wstore` for
# historical reasons even though callers correctly pass the id_store. Those are
# a naming wart, not a bug, and flagging them would make this gate noise.
report="$(grep -rnE \
    '(muxbus_load|muxbus_save|muxbus_clear|load_valid_token|ensure_agent_credential|relay_token)[[:space:]]*\(' \
    --include='*.rs' agentmux-srv/src \
    | grep 'state\.wstore' || true)"

if [[ -n "$report" ]]; then
    echo "ERROR: muxbus credential lookup against state.wstore." >&2
    echo "" >&2
    echo "muxbus credentials live in AppState::id_store — that is where" >&2
    echo "CloudSubscriber::init_global and the muxbus.login/status/disconnect" >&2
    echo "handlers write them. Reading from the per-channel state.wstore finds" >&2
    echo "nothing whenever the shared root resolves, and fails SILENTLY: the" >&2
    echo "lookup just returns None and the caller concludes the user is not" >&2
    echo "logged in." >&2
    echo "" >&2
    echo "$report" | sed 's/^/  - /' >&2
    echo "" >&2
    echo "Use state.id_store. See this script's header for why not" >&2
    echo "identity_store, and PR #3023 for the regression." >&2
    exit 1
fi

echo "OK: no muxbus credential lookup reads from state.wstore."
exit 0
