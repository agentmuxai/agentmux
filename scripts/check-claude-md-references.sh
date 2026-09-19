#!/usr/bin/env bash
# check-claude-md-references.sh — fail if a LIVING doc still points at the
# deleted repo-level CLAUDE.md.
#
# Why this is a script and not a grep in a spec:
# The port spec (SPEC_CLAUDE_MD_CONTENT_PORT_2026_09_18.md, landing
# separately) originally listed its verification steps as commands a human
# was expected to remember to run.
# That is the same gap the spec itself documents — a hand-run check matched
# markdown link syntax `](./CLAUDE.md)` only, reported "zero dangling", and
# missed plain-prose `see CLAUDE.md` references that review then caught.
# A check nobody runs is not a check.
#
# ---------------------------------------------------------------------------
# SCOPE — this gate covers LIVING documents only, and that is deliberate.
#
# A "living" doc is one a reader is expected to trust as current: README,
# CONTRIBUTING, BUILD, and the undated reference docs under docs/.
#
# NOT covered, and NOT failures:
#
#   1. Dated documents — any filename carrying a _YYYY_MM_DD / _YYYY-MM-DD
#      stamp, plus docs/{analysis,retro,reports,archive,research,incident,
#      investigations,status}/. These are point-in-time records. A reference
#      to a file that existed when they were written is CORRECT, and
#      rewriting them would falsify the record. This is the single biggest
#      exclusion: ~90 dated specs cite CLAUDE.md, every one legitimately.
#
#   2. Product behaviour. AgentMux generates CLAUDE.md files FOR managed
#      agents (agent_config.rs, bundles, provider startup files). That
#      feature still exists; text about it is not a reference to our file.
#
#   3. Documents whose subject IS the removal (this script, the port spec).
#
# If you are tempted to widen these exclusions to make a failure go away:
# don't. Repoint the reference instead. The exclusions describe documents
# that are allowed to be stale BY DESIGN, not documents that are merely
# inconvenient to fix.
#
# Usage: bash scripts/check-claude-md-references.sh

set -uo pipefail
cd "$(dirname "$0")/.."

SELF='scripts/check-claude-md-references.sh'
SPEC='docs/specs/SPEC_CLAUDE_MD_CONTENT_PORT_2026_09_18.md'

# Dated / historical documents whose references are correct as written.
EXCLUDE_PATH='(^(VERSION_HISTORY|CHANGELOG)\.md$)|(^docs/([a-z]+/)?(analysis|retro|reports|archive|research|incident|investigations|status)/)|([-_][0-9]{4}[-_][0-9]{2}[-_][0-9]{2}\.md$)'

# Lines about the CLAUDE.md files AgentMux writes for agents, not about ours.
# Every alternative must be CLAUDE.md-specific. Bare generic words (an earlier
# revision had 'bundle', 'provider', 'project config') silently skipped genuine
# references that merely happened to contain them -- exactly the bug this gate
# exists to catch. Review caught settings-cleanup.md slipping through on 'bundle'.
PRODUCT="generated .{0,2}claude\.md|generated per-agent|agent'?s? own claude\.md|per-agent claude|writes? [a-z ]{0,15}.{0,2}claude\.md|templates/host|\.claude/|claude_md_ownership|isolate_host_claude_md|agent-seed|agent_config\.rs|config file builder|startup instruction|global claude\.md|host claude\.md|workspace copy|filenamegroup|generates? [a-z ]{0,15}.{0,2}claude\.md|claude\.md[^.]{0,40}system prompt|system prompt[^.]{0,40}claude\.md|read a system prompt file"

fail=0
while IFS= read -r hit; do
    file=${hit%%:*}
    rest=${hit#*:}
    lineno=${rest%%:*}
    text=${rest#*:}

    [[ "$file" == "$SELF" || "$file" == "$SPEC" ]] && continue
    [[ "$file" =~ $EXCLUDE_PATH ]] && continue
    # The generated specs index repeats each spec's title. An entry whose
    # link target is itself an excluded (dated) document is that document's
    # text, not a living reference — judge it by its target, as above.
    if [[ "$file" == "docs/specs/INDEX.md" && "$text" =~ \]\(([^\)]+\.md)\) ]]; then
        [[ "docs/specs/${BASH_REMATCH[1]}" =~ $EXCLUDE_PATH ]] && continue
    fi
    shopt -s nocasematch
    if [[ "$text" =~ $PRODUCT ]]; then shopt -u nocasematch; continue; fi
    shopt -u nocasematch

    printf 'FAIL %s:%s\n' "$file" "$lineno"
    printf '     %.110s\n' "${text#"${text%%[![:space:]]*}"}"
    fail=1
done < <(git grep -n 'CLAUDE\.md' -- '*.md' 2>/dev/null)

if [[ $fail -ne 0 ]]; then
    cat <<'EOF'

check-claude-md-references: FAILED — a living document points at the deleted
repo-level CLAUDE.md.

Repoint it at whichever document now owns that content:

  architecture, widgets, isolation invariants  ->  README.md
  build, prerequisites, task commands          ->  BUILD.md
  git workflow, versioning, contribution rules ->  CONTRIBUTING.md
  log access                                   ->  docs/MUXLOG.md
  message security / sender identity           ->  README.md, docs/specs/SPEC_JEKT_*

...or just state the rule inline if it is a single line.

Do NOT silence this by adding the file to EXCLUDE_PATH. That list is for
documents allowed to be stale by design (dated records), not for ones that
are merely awkward to fix.

If the hit is about a CLAUDE.md that AgentMux GENERATES for an agent, it is
a false positive: reword the line to say so explicitly.
EOF
    exit 1
fi

echo "check-claude-md-references: ok"
