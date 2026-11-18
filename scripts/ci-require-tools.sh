#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Fail a CI step at once, with a clear message, when a tool it was meant to provide is not usable.
#
#   scripts/ci-require-tools.sh <tool>[=<version argument>] ...
#
# Each tool must be on PATH and must start: `<tool> --version` is run (another argument after `=`, or `=` alone to
# only look it up). `iscc` (the Inno Setup compiler) is also looked for in its install folders, because the installer
# does not put it on PATH; it is only looked up, since ISCC has no version flag.
#
# Why: a package manager that answers an error can still exit 0 (Chocolatey printed "installed 0/2 packages" and
# the step passed), so a missing tool surfaced minutes later as "command not found" from the build. Exit status 1
# lists every missing tool, so one run says all of it.

set -uo pipefail

[ "$#" -gt 0 ] || { echo "usage: ci-require-tools.sh <tool>[=<version arg>] ..." >&2; exit 2; }

missing=()
for spec in "$@"; do
    name="${spec%%=*}"
    if [[ "$spec" == *=* ]]; then arg="${spec#*=}"; else arg="--version"; fi

    if [ "$name" = iscc ]; then
        found="$(command -v iscc 2> /dev/null || command -v ISCC 2> /dev/null || true)"
        if [ -z "$found" ]; then
            for f in "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" "/c/Program Files/Inno Setup 6/ISCC.exe" \
                "${LOCALAPPDATA:-/nonexistent}/Programs/Inno Setup 6/ISCC.exe"; do
                if [ -f "$f" ]; then found="$f"; break; fi
            done
        fi
        if [ -n "$found" ]; then echo "  OK   iscc: $found"; else missing+=("iscc (Inno Setup 6 compiler: not on PATH and not in its install folders)"); fi
        continue
    fi

    found="$(command -v "$name" 2> /dev/null || true)"
    if [ -z "$found" ]; then missing+=("$name (not on PATH)"); continue; fi
    if [ -z "$arg" ]; then echo "  OK   $name: $found"; continue; fi
    if said="$("$name" "$arg" 2>&1)" && [ -n "$said" ]; then
        echo "  OK   $name: $found (${said%%$'\n'*})"
    else
        missing+=("$name ($found is on PATH but '$name $arg' failed)")
    fi
done

if [ "${#missing[@]}" -gt 0 ]; then
    echo "::error title=Build tool missing::the runner is missing a tool this job needs" >&2
    for m in "${missing[@]}"; do echo "MISSING  $m" >&2; done
    echo "A tool install step must provide these; do not let the build run on without them." >&2
    exit 1
fi
