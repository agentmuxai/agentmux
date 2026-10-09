#!/usr/bin/env bash
# weekly-web-link-report.sh — turn lychee's Markdown output into the comment
# the weekly web-link report posts (.github/workflows/weekly-web-links.yml).
#
#   scripts/weekly-web-link-report.sh <lychee-out.md> <lychee-exit-code> <links-dir> <out-dir>
#
# Writes <out-dir>/report.md and <out-dir>/fingerprint.txt (the failing URLs
# and their statuses, without positions, so
# scripts/docs-stale-issue-comment.sh posts again only when the failures
# change). <links-dir> is the mirror scripts/collect-web-links.mjs wrote; its
# prefix and the `.txt` suffix are stripped so the report names repo files.
# A REPORT: always exits 0.
set -uo pipefail

lychee_out="${1:?usage: weekly-web-link-report.sh <lychee-out.md> <exit-code> <links-dir> <out-dir>}"
code="${2:-}"
links_dir="${3:?missing links dir}"
out="${4:?missing out dir}"
mkdir -p "$out"

run_url="${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-}/actions/runs/${GITHUB_RUN_ID:-}"
footer="_Run: ${run_url} · \`weekly-web-links.yml\` checks github.com and agentmux.ai links in docs/ and code comments, and posts only when the failures change._"
# lychee: 0 = all links fine, 2 = some links failed; anything else is lychee
# itself failing (bad config, crash), which says nothing about the links.
if [ ! -s "$lychee_out" ] || { [ "$code" != 0 ] && [ "$code" != 2 ]; }; then
  {
    echo "### Weekly web-link report"
    echo
    echo ":warning: lychee did not run to completion (exit code \`${code:-unknown}\`); see the run log."
    echo
    echo "$footer"
  } > "$out/report.md"
  echo "lychee-failed ${code:-unknown}" > "$out/fingerprint.txt"
  exit 0
fi

# `### Errors in web-links/docs/x.md.txt` -> `### Errors in docs/x.md`.
prefix="${links_dir%/}/"
sed -e "/^#* *Errors in /{s#${prefix}##;s#\.txt\$##}" "$lychee_out" > "$out/lychee.md"
grep -oE '^\* \[[^]]*\] <[^>]+>' "$out/lychee.md" | sort -u > "$out/fingerprint.txt"
failures=$(wc -l < "$out/fingerprint.txt" | tr -d ' ')

{
  echo "### Weekly web-link report"
  echo
  if [ "$failures" = 0 ]; then
    total=$(grep -E 'Total *\| *[0-9]+' "$out/lychee.md" | grep -oE '[0-9]+' | tail -n 1)
    echo "All clear: ${total:-every} checked link(s) resolved."
  else
    echo "**$failures** link(s) failed. Fix the link, or drop it if the target is gone for good. \`(at LINE:COL)\` is the line in that file."
    echo
    # lychee's own summary table and per-file errors, one heading level down.
    sed -e 's/^# /#### /' -e 's/^## /#### /' -e 's/^### /##### /' "$out/lychee.md"
  fi
  echo
  echo "$footer"
} > "$out/report.md"
exit 0
