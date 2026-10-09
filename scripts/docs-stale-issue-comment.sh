#!/usr/bin/env bash
# docs-stale-issue-comment.sh — post a report as a comment on the standing
# docs-stale issue that .github/workflows/docs-stale-sweep.yml keeps.
#
#   scripts/docs-stale-issue-comment.sh <key> <body-file> <fingerprint-file>
#
# The sweep owns the issue BODY (it rewrites it weekly). Other reports
# (the nightly doc-link report, the weekly web-link report) add COMMENTS, so
# they never overwrite the sweep or each other.
#
# A comment is posted only when the findings change: the hash of
# <fingerprint-file> is stored in a hidden marker, and if the newest comment
# with the same <key> carries the same hash, nothing is posted. The
# fingerprint file should hold the findings without volatile detail (line
# numbers, timestamps), so an unrelated edit does not re-post the same list.
#
# If no open issue exists (closed after triage, or never created), one is
# created with the sweep's title and label, and the sweep fills its body on
# its next run. ISSUE_TITLE / ISSUE_LABEL default to the sweep's values and
# must stay in step with docs-stale-sweep.yml.
#
# Needs GH_TOKEN (issues: write) and GH_REPO. Reporting only: callers run it
# with continue-on-error, so a failure here never fails a build.
set -euo pipefail

key="${1:?usage: docs-stale-issue-comment.sh <key> <body-file> <fingerprint-file>}"
body_file="${2:?missing body file}"
fp_file="${3:?missing fingerprint file}"
: "${GH_REPO:?GH_REPO must be set}"
ISSUE_TITLE="${ISSUE_TITLE:-Docs stale-sweep triage (auto-updated weekly)}"
ISSUE_LABEL="${ISSUE_LABEL:-docs-stale-sweep}"
# GitHub rejects comments over 65,536 characters.
MAX_BYTES=60000

fp=$(sha256sum < "$fp_file" | cut -c1-16)
marker_prefix="<!-- docs-stale-report:${key} fp="

issue=$(gh issue list --label "$ISSUE_LABEL" --state open --json number --jq '.[0].number // empty')
if [ -z "$issue" ]; then
  gh label create "$ISSUE_LABEL" --force \
    --description 'Auto-updated weekly by docs-stale-sweep.yml' --color 'c5def5'
  url=$(gh issue create --title "$ISSUE_TITLE" --label "$ISSUE_LABEL" \
    --body "Standing docs-stale issue. The weekly sweep (docs-stale-sweep.yml) fills this body; other doc reports post as comments.")
  issue="${url##*/}"
  echo "created issue #$issue"
fi

last_fp=$(gh api --paginate "repos/${GH_REPO}/issues/${issue}/comments" \
  --jq ".[].body | select(contains(\"${marker_prefix}\"))" \
  | grep -oE "docs-stale-report:${key} fp=[0-9a-f]+" | tail -n 1 | sed 's/.*fp=//' || true)
if [ "$last_fp" = "$fp" ]; then
  echo "findings unchanged since the last ${key} comment on #${issue} (fp=${fp}); not posting"
  exit 0
fi

out=$(mktemp)
if [ "$(wc -c < "$body_file")" -gt "$MAX_BYTES" ]; then
  head -c "$MAX_BYTES" "$body_file" > "$out"
  printf '\n\n_(truncated; the full report is in the run summary and artifact)_\n' >> "$out"
else
  cat "$body_file" > "$out"
fi
printf '\n%s%s -->\n' "$marker_prefix" "$fp" >> "$out"
gh issue comment "$issue" --body-file "$out"
echo "posted ${key} report to #${issue} (fp=${fp})"
