#!/usr/bin/env bash
# check-hash-from-stdin.sh — CI grep gate: hash files from stdin, never by name.
#
# docs/specs/SPEC_WINDOWS_CEF_RUNTIME_VERIFY_BACKSLASH_PATH_HASH_2026_09_24.md
#
# THE RULE: when a script parses a checksum out of `sha256sum`-family output,
# it must feed the file on stdin:
#
#     actual="$(sha256sum < "$f" | cut -d' ' -f1)"      # ok
#     actual="$(sha256sum "$f" | cut -d' ' -f1)"        # FAILS this gate
#
# Given a file NAME containing a backslash, GNU coreutils escapes it and
# prefixes the whole output line -- hash included -- with `\`. On Windows every
# path can contain one (Task's `$HOME` is C:\Users\<user>; CI's workspace is
# D:\a\...), so the parsed hash never matches its pin. That is how
# verify-cef-runtime-windows.sh refused the correct r2 runtime on 2026-09-23.
# From stdin there is no name to escape.
#
# Flags a hash command given a file argument (after any options) whose output
# is piped on the same line. Stdin forms (`sha256sum < f | cut`,
# `printf … | shasum -a 1 | cut`) and non-parsing uses (`sha256sum -c`, a hash
# printed for a human) pass. Comment lines are ignored.
#
# Usage:
#   bash scripts/check-hash-from-stdin.sh             # scan the repo's scripts
#   bash scripts/check-hash-from-stdin.sh <file>...   # scan these files only
# Exit 0 = clean, exit 1 = a violation was found.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

if [ "$#" -gt 0 ]; then
    files=("$@")
else
    mapfile -t files < <(
        {
            git ls-files -- 'scripts/*.sh' 'scripts/**/*.sh' 'tools/*.sh' 'tools/**/*.sh' \
                '.github/workflows/*.yml' '.github/workflows/*.yaml' 'Taskfile.yml' 'Taskfile*.yml'
        } 2>/dev/null | sort -u | grep -v -e '^scripts/check-hash-from-stdin\.sh$' -e '^scripts/check-hash-from-stdin\.test\.sh$' || true
    )
fi

# grep with no file arguments would read stdin -- never let an empty list hang.
if [ "${#files[@]}" -eq 0 ]; then
    echo "check-hash-from-stdin: no files to scan"
    exit 0
fi

# <hash-cmd> [option [numeric value]]... <file arg> ... | ...
# The file arg may not start with `-`, `<`, `|`, or a digit -- the digit rule
# makes `shasum -a 1 | cut` read `1` as the option's value, not a file name.
pattern='(^|[^[:alnum:]_-])(sha(1|224|256|384|512)sum|md5sum|b2sum|shasum)([[:space:]]+-[[:alnum:]-]+([[:space:]]+[0-9]+)?)*[[:space:]]+[^-<|[:space:][:digit:]][^|]*\|'

report="$(grep -nHE "$pattern" "${files[@]}" 2>/dev/null | grep -vE '^[^:]+:[0-9]+:[[:space:]]*#' || true)"

if [ -n "$report" ]; then
    echo "check-hash-from-stdin: FAILED — hash parsed from a file-name argument:" >&2
    echo "$report" | sed 's/^/  /' >&2
    echo "" >&2
    echo "  Feed the file on stdin instead: sha256sum < \"\$f\" | cut -d' ' -f1" >&2
    echo "  (a file name containing '\\' makes coreutils prefix the hash with '\\')." >&2
    echo "  See docs/specs/SPEC_WINDOWS_CEF_RUNTIME_VERIFY_BACKSLASH_PATH_HASH_2026_09_24.md" >&2
    exit 1
fi

echo "check-hash-from-stdin: ok (${#files[@]} file(s) scanned)"
