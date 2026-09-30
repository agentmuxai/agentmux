#!/usr/bin/env bash
# check-no-cef-version-in-overview.sh — CI grep gate.
#
# The README and the high-level diagrams under assets/ describe the
# architecture, not a release. They used to name the CEF/Chromium milestone
# ("Chromium 152"), so every CEF upgrade had to edit the README and redraw
# assets/architecture.svg by hand, and they went stale whenever it didn't.
# The pinned version lives in scripts/cef-build/cef-runtime-pins.sh and the
# app's version panel shows it; the overview says "Chromium via CEF".
#
# Fails if README.md or any assets/*.svg names a three-digit CEF or Chromium
# version. Dated records (specs, retros, VERSION_HISTORY.md) are not checked.
set -euo pipefail
cd "$(dirname "$0")/.."

violations=$(grep -nEi '\b(cef|chromium)[ -]?v?[0-9]{3}\b' README.md assets/*.svg 2>/dev/null || true)

if [[ -n "$violations" ]]; then
    echo "check-no-cef-version-in-overview: a CEF/Chromium version is named in the overview:" >&2
    echo "$violations" >&2
    echo "" >&2
    echo "Write \"Chromium via CEF\" instead. The pinned version belongs in" >&2
    echo "scripts/cef-build/cef-runtime-pins.sh, not in the README or diagrams." >&2
    exit 1
fi
echo "check-no-cef-version-in-overview: ok"
