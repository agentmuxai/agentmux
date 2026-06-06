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
CHANNEL="${CHANNEL:-dev-portable-main-b28b7a}"
CHANNEL_DIR="$AGENTMUX_DIR/channels/$CHANNEL"

# ── helpers ──────────────────────────────────────────────────────────────────

db_query() { sqlite3 "$1" "$2" 2>/dev/null; }

# sqlite3 ATTACH fails on long POSIX paths on Windows; convert to Win32
win_path() { cygpath -w "$1" 2>/dev/null || echo "$1"; }

# ms-epoch timestamp
now_ms() { date +%s%3N; }

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
            names_in=$(echo "$name_filter" | tr ',' '\n' | \
                awk '{printf "%s'\''%s'\''", (NR>1?",":""), $0}')
            where="is_seeded=0 AND name IN ($names_in)"
        else
            where="is_seeded=0"
        fi

        agents=$(db_query "$src_db" \
            "SELECT id, name FROM db_agent_definitions WHERE $where;" 2>/dev/null)
        [ -z "$agents" ] && continue

        local src_win; src_win=$(win_path "$src_db")

        while IFS='|' read -r def_id agent_name; do
            # Skip if definition already present
            exists=$(db_query "$target_db" \
                "SELECT count(*) FROM db_agent_definitions WHERE id='$def_id';")
            if [ "$exists" -gt 0 ]; then
                echo "  SKIP  $agent_name (already in target)"
                ((skipped++)) || true
                continue
            fi

            # Copy definition
            sqlite3 "$src_win" \
                "ATTACH '$tgt_win' AS dst;
                 INSERT OR IGNORE INTO dst.db_agent_definitions
                 SELECT * FROM db_agent_definitions WHERE id='$def_id';"

            # Create a stub instance so the agent appears in My Agents
            # (ListRecentSessionsCommand queries instances, not definitions)
            local stub_id="stub-$(echo "$def_id" | tr -dc 'a-z0-9' | head -c8)-01"
            sqlite3 "$tgt_win" \
                "INSERT OR IGNORE INTO db_agent_instances
                   (id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, identity_id, memory_id, instance_name,
                    working_directory, display_hidden, started_at, ended_at, created_at)
                 VALUES
                   ('$stub_id', '$def_id', '', '', '', 'stopped',
                    '', '', '', '$agent_name',
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
