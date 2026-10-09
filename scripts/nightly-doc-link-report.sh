#!/usr/bin/env bash
# nightly-doc-link-report.sh — the nightly's repo-wide doc-link report.
# Plan step 3 of docs/specs/PLAN_CI_TEST_SPEED_AND_DRY_FOLLOWUPS_2026_10_09.md.
#
#   scripts/nightly-doc-link-report.sh <out-dir>
#
# Runs `check-doc-links.mjs --all` (broken relative Markdown links) and
# `check-comment-hygiene.mjs --dead-refs` (comments naming a file that is not
# in the repo) over the whole tree and writes:
#   <out-dir>/report.md        the Markdown report
#   <out-dir>/fingerprint.txt  the findings without line numbers, so
#                              scripts/docs-stale-issue-comment.sh posts again
#                              only when the findings themselves change
# plus the raw outputs. A REPORT, never a gate: it always exits 0, and a
# checker that fails to run is named in the report instead.
set -uo pipefail

out="${1:?usage: nightly-doc-link-report.sh <out-dir>}"
mkdir -p "$out"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root" || exit 0

errors=()

node scripts/check-doc-links.mjs --all > "$out/links.txt" 2>&1
[ $? -eq 0 ] || errors+=("check-doc-links.mjs --all exited non-zero")
broken=$(grep -oE '[0-9]+ broken in' "$out/links.txt" | grep -oE '^[0-9]+' | tail -n 1)
if [ -z "$broken" ]; then
  broken='?'
  errors+=("check-doc-links.mjs printed no summary line")
fi
sed -n '/^Broken relative Markdown links:/,/^Fix the path/p' "$out/links.txt" \
  | grep -vE '^(Broken relative Markdown links:|Fix the path|[[:space:]]*$)' > "$out/links-list.txt"

if ! node scripts/check-comment-hygiene.mjs --dead-refs --json > "$out/dead-refs.json" 2> "$out/dead-refs.err" \
  || ! jq -e 'type == "array"' "$out/dead-refs.json" > /dev/null 2>&1; then
  errors+=("check-comment-hygiene.mjs --dead-refs failed")
  echo '[]' > "$out/dead-refs.json"
fi
# A comment that opted out (`comment-hygiene: allow`) is deliberate; skip it.
jq '[.[] | select(.allowed | not)]' "$out/dead-refs.json" > "$out/dead.json"
total=$(jq 'length' "$out/dead.json")
bare=$(jq '[.[] | select(.kind == "bare")] | length' "$out/dead.json")
strong=$((total - bare))
jq -r '.[] | select(.kind != "bare") | "\(.file):\(.line): [\(.kind)] \(.ref)"' "$out/dead.json" > "$out/dead-strong.txt"
jq -r '.[] | select(.kind == "bare") | "\(.file):\(.line): \(.ref)"' "$out/dead.json" > "$out/dead-bare.txt"

{
  cat "$out/links-list.txt"
  jq -r '.[] | "\(.file) [\(.kind)] \(.ref)"' "$out/dead.json" | sort -u
  printf '%s\n' "${errors[@]}"
} > "$out/fingerprint.txt"

run_url="${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-}/actions/runs/${GITHUB_RUN_ID:-}"
{
  echo "### Nightly doc-link report"
  echo
  for e in "${errors[@]}"; do echo "- :warning: $e (see the run log)"; done
  if [ "$broken" = 0 ] && [ "$total" = 0 ] && [ ${#errors[@]} -eq 0 ]; then
    echo "All clear: no broken relative doc links, and no comment names a missing file."
  else
    echo "- Broken relative doc links (\`check-doc-links.mjs --all\`): **$broken**"
    echo "- Comments naming a missing file (\`check-comment-hygiene.mjs --dead-refs\`): **$total** ($strong doc or repo path, $bare bare name)"
    if [ -s "$out/links-list.txt" ]; then
      echo
      echo '```'
      cat "$out/links-list.txt"
      echo '```'
    fi
    if [ -s "$out/dead-strong.txt" ]; then
      echo
      echo "Doc names and repo paths (the kinds the PR gate fails on an added line):"
      echo
      echo '```'
      cat "$out/dead-strong.txt"
      echo '```'
    fi
    if [ -s "$out/dead-bare.txt" ]; then
      echo
      echo "<details><summary>$bare bare-name references (lower signal: many are examples or other repos)</summary>"
      echo
      echo '```'
      cat "$out/dead-bare.txt"
      echo '```'
      echo
      echo '</details>'
    fi
  fi
  echo
  echo "_Run: ${run_url} · \`ci-nightly-build.yml\` posts this only when the findings change._"
} > "$out/report.md"
exit 0
