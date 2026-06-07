#!/usr/bin/env bash
# import-agents.sh — copy user-created agent definitions into a target version DB
# and create stub session instances so they appear in My Agents without a restart.
#
# Usage:
#   scripts/import-agents.sh                        # catalog all agents across all DBs
#   scripts/import-agents.sh --to 0.43.2            # import all user agents into version
#   scripts/import-agents.sh --to 0.43.2 --name Maks,Maksi
#
# After running, reopen the agent pane in the target build (no full restart needed).
# The agents appear in My Agents with status "stopped", ready to launch.

set -euo pipefail

AGENTMUX_DIR="${AGENTMUX_DIR:-$HOME/.agentmux}"

# Auto-detect channel from the current git branch when CHANNEL is not set.
# The portable build channel name is: dev-portable-<branch>-<sha1(branch)[:6]>
# where sha1 is of the branch name string itself (not the commit).
if [ -z "${CHANNEL:-}" ]; then
    _branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "main")
    _branch_hash=$(printf '%s' "$_branch" | sha1sum | cut -c1-6 2>/dev/null \
                   || printf '%s' "$_branch" | shasum | cut -c1-6)
    CHANNEL="dev-portable-${_branch}-${_branch_hash}"
fi
CHANNEL_DIR="$AGENTMUX_DIR/channels/$CHANNEL"

# ── helpers ──────────────────────────────────────────────────────────────────

db_query() { sqlite3 "$1" "$2" 2>/dev/null; }

# sqlite3 ATTACH fails on long POSIX paths on Windows; convert to Win32
win_path() { cygpath -w "$1" 2>/dev/null || echo "$1"; }

# ms-epoch timestamp — uses epoch seconds × 1000; %N is a GNU extension and
# is not available on macOS/BSD, so we avoid it for portability.
now_ms() { printf '%s000\n' "$(date +%s)"; }

# Find every objects.db under the agentmux dir
all_dbs() {
    find "$AGENTMUX_DIR" -name "objects.db" 2>/dev/null | sort
}

# ── catalog ──────────────────────────────────────────────────────────────────

catalog() {
    echo ""
    echo "User-created agents across all data dirs:"
    echo "──────────────────────────────────────────────────────────"
    printf "%-20s %-12s %-10s %s\n" "NAME" "PROVIDER" "VERSION" "SOURCE"
    echo "──────────────────────────────────────────────────────────"
    while IFS= read -r db; do
        result=$(db_query "$db" \
            "SELECT name, provider FROM db_agent_definitions WHERE is_seeded=0 ORDER BY name;")
        [ -z "$result" ] && continue
        label=$(echo "$db" | sed \
            -e "s|$AGENTMUX_DIR/||" \
            -e 's|/data/db/objects\.db||' \
            -e 's|channels/||' \
            -e 's|versions/||')
        version=$(echo "$label" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)
        version="${version:-dev}"
        while IFS='|' read -r name provider; do
            printf "%-20s %-12s %-10s %s\n" "$name" "$provider" "$version" "$label"
        done <<< "$result"
    done < <(all_dbs)
    echo ""
    echo "Run with --to <version> to import into that version."
    echo "Example: scripts/import-agents.sh --to 0.43.2"
    echo "         scripts/import-agents.sh --to 0.43.2 --name Maks,Maksi"
}

# ── import ───────────────────────────────────────────────────────────────────

import_into() {
    local target_version="$1"
    local name_filter="$2"   # comma-separated names, or "" for all
    local target_db="$CHANNEL_DIR/versions/$target_version/data/db/objects.db"

    if [ ! -f "$target_db" ]; then
        echo "ERROR: Target DB not found: $target_db" >&2
        echo "       Build and launch version $target_version at least once first." >&2
        echo "" >&2
        echo "       If you're using a different channel, pass --channel <name>." >&2
        echo "       Available channels: $(ls "$AGENTMUX_DIR/channels/" 2>/dev/null | tr '\n' '  ')" >&2
        exit 1
    fi

    local tgt_win; tgt_win=$(win_path "$target_db")
    local ts; ts=$(now_ms)
    local imported=0 skipped=0

    echo "Target: $target_db"
    echo ""

    while IFS= read -r src_db; do
        [ "$src_db" = "$target_db" ] && continue

        if [ -n "$name_filter" ]; then
            # Escape apostrophes in each name before building the SQL IN list.
            names_in=$(echo "$name_filter" | tr ',' '\n' | \
                sed "s/'/''/g" | \
                awk '{printf "%s'\''%s'\''", (NR>1?",":""), $0}')
            where="is_seeded=0 AND name IN ($names_in)"
        else
            where="is_seeded=0"
        fi

        # Use char(1) (ASCII unit-separator) instead of '|' so agent names that
        # contain a pipe character don't corrupt the field split.
        agents=$(db_query "$src_db" \
            "SELECT id||char(1)||name FROM db_agent_definitions WHERE $where;" 2>/dev/null)
        [ -z "$agents" ] && continue

        local src_win; src_win=$(win_path "$src_db")

        while IFS=$'\x01' read -r def_id agent_name; do
            # Escape single-quotes for safe SQL interpolation (e.g. "Bob's Agent" → "Bob''s Agent")
            local safe_name="${agent_name//\'/\'\'}"

            # Skip if definition already present
            exists=$(db_query "$target_db" \
                "SELECT count(*) FROM db_agent_definitions WHERE id='$def_id';")
            if [ "$exists" -gt 0 ]; then
                echo "  SKIP  $agent_name (already in target)"
                ((skipped++)) || true
                continue
            fi

            # Copy definition row (always required).
            # Explicit column list avoids INSERT/SELECT * schema-mismatch failures
            # when importing across versions with different db_agent_definitions schemas.
            sqlite3 "$src_win" \
                "ATTACH '$tgt_win' AS dst;
                 INSERT OR IGNORE INTO dst.db_agent_definitions
                   (id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start, restart_on_crash,
                    idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id,
                    is_seeded, accounts, parent_id, branch_label, updated_at, user_hidden)
                 SELECT
                    id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start, restart_on_crash,
                    idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id,
                    is_seeded, accounts, parent_id, branch_label, updated_at, user_hidden
                 FROM db_agent_definitions WHERE id='$def_id';"

            # Also populate the consolidated db_agents table (schema v4+).
            # agent_def_list() and instance_list() read db_agents; skipping this
            # write leaves the agent invisible to the running app even though the
            # definition row is present. Silent no-op on pre-v4 targets where the
            # table does not exist yet.
            sqlite3 "$src_win" \
                "ATTACH '$tgt_win' AS dst;
                 INSERT OR IGNORE INTO dst.db_agents
                   (id, name, icon, description,
                    is_template, parent_template_id,
                    provider, provider_flags, shell, environment, working_directory,
                    agent_type, agent_bus_id, accounts,
                    auto_start, restart_on_crash, idle_timeout_minutes,
                    slug, branch_label,
                    created_at, updated_at, is_seeded, user_hidden)
                 SELECT
                   id, name, icon, description,
                   0,  parent_id,
                   provider, provider_flags, shell, environment, working_directory,
                   agent_type, agent_bus_id, accounts,
                   auto_start, restart_on_crash, idle_timeout_minutes,
                   slug, branch_label,
                   created_at, updated_at, is_seeded, user_hidden
                 FROM db_agent_definitions WHERE id='$def_id';" 2>/dev/null || true

            # Create a stub instance so the agent appears in My Agents
            # (ListRecentSessionsCommand queries instances, not definitions).
            # Derive stub_id from the full def_id (UUID, globally unique) so
            # repeated imports are idempotent and INSERT OR IGNORE is safe.
            local stub_id="si-$(echo "$def_id" | tr -d '-')"
            sqlite3 "$tgt_win" \
                "INSERT OR IGNORE INTO db_agent_instances
                   (id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, identity_id, memory_id, instance_name,
                    working_directory, display_hidden, started_at, ended_at, created_at)
                 VALUES
                   ('$stub_id', '$def_id', '', '', '', 'stopped',
                    '', '', '', '$safe_name',
                    '', 0, $ts, 0, $ts);"

            echo "  OK    $agent_name"
            ((imported++)) || true
        done <<< "$agents"
    done < <(all_dbs)

    echo ""
    echo "Imported: $imported  Skipped: $skipped"
    [ "$imported" -gt 0 ] && \
        echo "Reopen the agent pane in the $target_version build to see the changes."
}

# ── arg parsing ──────────────────────────────────────────────────────────────

TARGET_VERSION=""
NAME_FILTER=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --to)      TARGET_VERSION="$2"; shift 2 ;;
        --name)    NAME_FILTER="$2";    shift 2 ;;
        --channel) CHANNEL="$2"; CHANNEL_DIR="$AGENTMUX_DIR/channels/$CHANNEL"; shift 2 ;;
        *) echo "Unknown arg: $1" >&2; exit 1 ;;
    esac
done

if [ -n "$TARGET_VERSION" ]; then
    import_into "$TARGET_VERSION" "$NAME_FILTER"
else
    catalog
fi
