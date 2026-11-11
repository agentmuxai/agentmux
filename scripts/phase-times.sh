#!/usr/bin/env bash
# Per-phase times for a packaging script, sourced by it (scripts/package-macos.sh).
#
#   . "$REPO_ROOT/scripts/phase-times.sh"
#   task build:frontend; phase_done "frontend"
#
# Each `phase_done NAME` logs the time since the previous mark and keeps it. On exit, even a failed one,
# the table goes to the log and, in CI, to the job summary, so growth shows up release by release and
# a build that stops part-way still shows the phases it finished.

PHASE_MARK=$SECONDS
PHASE_START=$SECONDS
PHASE_ROWS=""

phase_fmt() {
    echo "$(($1 / 60))m$(printf '%02d' $(($1 % 60)))s"
}

phase_done() {
    local now=$SECONDS
    local took=$((now - PHASE_MARK))
    PHASE_MARK=$now
    PHASE_ROWS="${PHASE_ROWS}| $1 | $(phase_fmt "$took") |"$'\n'
    echo "==> phase done: $1 in $(phase_fmt "$took")"
}

phase_summary() {
    [ -n "$PHASE_ROWS" ] || return 0
    local table
    table="| phase | time |"$'\n'"|---|---|"$'\n'"${PHASE_ROWS}| total | $(phase_fmt $((SECONDS - PHASE_START))) |"
    echo "==> phase times"
    echo "$table"
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        { echo "### Build phases"; echo; echo "$table"; echo; } >> "$GITHUB_STEP_SUMMARY" || true
    fi
}

trap phase_summary EXIT
