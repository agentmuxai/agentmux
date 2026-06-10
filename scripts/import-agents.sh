#!/usr/bin/env bash
# import-agents.sh — copy user-created agent definitions into a target version DB,
# create session instances so they appear in My Agents without a restart, AND wire
# each instance to its existing on-disk Claude session so its real (long)
# conversation resumes on the next message.
#
# The conversation transcript itself is NOT stored in objects.db — it lives in the
# provider CLI's account-wide session file (~/.claude/projects/<cwd>/<sid>.jsonl),
# which the target instance can already read. So "copy all data" means wiring the
# instance row to that session (session_id + working_directory), not copying bytes.
# Sending the imported agent one message re-spawns it with `--resume <sid>` and the
# full history replays into the pane.
#
# Usage:
#   scripts/import-agents.sh                        # catalog all agents across all DBs
#   scripts/import-agents.sh --to 0.43.2            # import all user agents into version
#   scripts/import-agents.sh --to 0.43.2 --name Maks,Maksi
#   scripts/import-agents.sh --to 0.43.1 --channel local-foo-abc123
#
# After running, reopen the agent pane in the target build (no full restart needed).
# Agents appear in My Agents as "stopped"; send one a message to resume its
# conversation. Agents with no on-disk session import as before (empty stub).

set -euo pipefail

AGENTMUX_DIR="${AGENTMUX_DIR:-$HOME/.agentmux}"
CHANNEL="${CHANNEL:-}"
# Pre-compute CHANNEL_DIR when CHANNEL is supplied via env var or --channel.
# import_into() sets both when it auto-detects the channel.
CHANNEL_DIR="${CHANNEL:+$AGENTMUX_DIR/channels/$CHANNEL}"

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

# ── session linkage ──────────────────────────────────────────────────────────
# AgentMux stores nothing of the conversation transcript in objects.db; the
# document the agent pane renders comes from the provider CLI's own session
# file, replayed when the agent is re-spawned with `--resume <session_id>`.
# For Claude Code that lives at:
#   ~/.claude/projects/<slugified-cwd>/<session_id>.jsonl   (account-wide)
# where <slugified-cwd> is the agent's working directory with each of : \ . /
# replaced by '-'. Agent working dirs follow the convention
# ~/.agentmux/agents/<slug>-<id>. So to make an imported agent show its real
# (long) conversation, we don't copy transcript bytes — we wire the instance
# row to the existing on-disk session, and a single message resumes it.
CLAUDE_PROJECTS="${CLAUDE_PROJECTS:-$HOME/.claude/projects}"
AGENTS_DIR="$AGENTMUX_DIR/agents"

# Slugify a working dir the way Claude Code names its project subdir.
slugify_cwd() { printf '%s' "$1" | sed 's#[:\\./]#-#g'; }

# Resolve the LARGEST Claude session for an agent slug (longest transcript →
# best streaming/perf repro). Echoes "<session_id>\t<working_directory_win>"
# (tab-separated) or nothing if no session is found.
best_session_for_slug() {
    local slug="$1"
    [ -n "$slug" ] || return 0
    local best_size=0 best_sid="" best_wd=""
    local d name wd_win cslug jf sz
    for d in "$AGENTS_DIR/$slug"-*; do
        [ -d "$d" ] || continue
        wd_win=$(win_path "$d")               # C:\Users\...\.agentmux\agents\<name>
        cslug=$(slugify_cwd "$wd_win")
        [ -d "$CLAUDE_PROJECTS/$cslug" ] || continue
        while IFS= read -r jf; do
            [ -n "$jf" ] || continue
            # `wc -c` is POSIX; `stat -c%s` is GNU-only and fails silently on
            # BSD/macOS — that would make every size 0 and no-op the wiring.
            sz=$(wc -c < "$jf" 2>/dev/null | tr -d '[:space:]'); sz=${sz:-0}
            if [ "$sz" -gt "$best_size" ]; then
                best_size=$sz
                best_sid=$(basename "$jf" .jsonl)
                best_wd=$wd_win
            fi
        done < <(find "$CLAUDE_PROJECTS/$cslug" -maxdepth 1 -name '*.jsonl' 2>/dev/null)
    done
    [ -n "$best_sid" ] && printf '%s\t%s' "$best_sid" "$best_wd"
}

# Scan $AGENTMUX_DIR/channels for the channel that contains a given version's
# data dir. More robust than deriving the channel name from the current git
# branch (which breaks after a channel rename or when the user has switched
# branches since the build was created).
detect_channel_for_version() {
    local version="$1"
    # Path depth from $AGENTMUX_DIR/channels:
    #   <channel>/versions/<version>/data/db/objects.db  → 6 levels
    find "$AGENTMUX_DIR/channels" -maxdepth 6 \
        -path "*/versions/$version/data/db/objects.db" 2>/dev/null | \
        head -1 | sed -e "s|$AGENTMUX_DIR/channels/||" -e 's|/versions/.*||'
}

# ── catalog ──────────────────────────────────────────────────────────────────

catalog() {
    echo ""
    echo "User-created agents across all data dirs:"
    echo "──────────────────────────────────────────────────────────"
    printf "%-20s %-12s %-10s %s\n" "NAME" "PROVIDER" "VERSION" "SOURCE"
    echo "──────────────────────────────────────────────────────────"
    while IFS= read -r db; do
        # Use char(1) (ASCII unit-separator) as column delimiter so agent
        # names containing '|' don't corrupt the field split.
        result=$(db_query "$db" \
            "SELECT name||char(1)||provider FROM db_agent_definitions WHERE is_seeded=0 ORDER BY name;")
        [ -z "$result" ] && continue
        label=$(echo "$db" | sed \
            -e "s|$AGENTMUX_DIR/||" \
            -e 's|/data/db/objects\.db||' \
            -e 's|channels/||' \
            -e 's|versions/||')
        version=$(echo "$label" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)
        version="${version:-dev}"
        while IFS=$'\x01' read -r name provider; do
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

    # Auto-detect channel by scanning existing version dirs if not specified.
    if [ -z "$CHANNEL" ]; then
        CHANNEL=$(detect_channel_for_version "$target_version")
        if [ -z "$CHANNEL" ]; then
            echo "ERROR: No channel found containing version $target_version." >&2
            echo "       Launch version $target_version at least once, or pass --channel." >&2
            echo "       Available channels: $(ls "$AGENTMUX_DIR/channels/" 2>/dev/null | tr '\n' '  ')" >&2
            exit 1
        fi
        CHANNEL_DIR="$AGENTMUX_DIR/channels/$CHANNEL"
    fi

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
            "SELECT id||char(1)||name||char(1)||COALESCE(slug,'') FROM db_agent_definitions WHERE $where;" 2>/dev/null)
        [ -z "$agents" ] && continue

        local src_win; src_win=$(win_path "$src_db")

        # Detect which newer columns exist in the source schema so the SELECT
        # does not fail on older AgentMux databases that lack them.
        local _src_cols; _src_cols=$(sqlite3 "$src_win" \
            "SELECT group_concat(name) FROM pragma_table_info('db_agent_definitions');" \
            2>/dev/null || echo "")
        local _col_updated_at _col_user_hidden
        if echo "$_src_cols" | grep -qw "updated_at"; then
            _col_updated_at="updated_at"
        else
            _col_updated_at="created_at"
        fi
        if echo "$_src_cols" | grep -qw "user_hidden"; then
            _col_user_hidden="user_hidden"
        else
            _col_user_hidden="0"
        fi

        while IFS=$'\x01' read -r def_id agent_name agent_slug; do
            # Escape single-quotes for safe SQL interpolation (e.g. "Bob's Agent" → "Bob''s Agent")
            local safe_name="${agent_name//\'/\'\'}"

            # Note whether the definition is already present. We do NOT skip the
            # agent in that case: an earlier run may have created the definition
            # plus an empty-session stub, and we still want to backfill the real
            # session linkage below. Every COPY below is INSERT OR IGNORE, so
            # re-running them on an existing agent is a harmless no-op.
            local def_exists=0
            if [ "$(db_query "$target_db" \
                "SELECT count(*) FROM db_agent_definitions WHERE id='$def_id';")" -gt 0 ]; then
                def_exists=1
            fi

            # Copy definition row (always required).
            # Explicit column list avoids INSERT/SELECT * schema-mismatch failures
            # when importing across versions with different db_agent_definitions schemas.
            # Uses detected column names so source DBs lacking updated_at/user_hidden
            # fall back to created_at/0 rather than aborting the import.
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
                    is_seeded, accounts, parent_id, branch_label,
                    ${_col_updated_at}, ${_col_user_hidden}
                 FROM db_agent_definitions WHERE id='$def_id';"

            # Verify the definition was actually inserted.
            # db_agent_definitions has a UNIQUE index on slug — an INSERT OR IGNORE
            # silently skips when the target already has a different agent with the
            # same slug. Creating a stub for a missing definition would leave a
            # broken My Agents entry, so abort this agent if the row didn't land.
            local def_landed; def_landed=$(db_query "$target_db" \
                "SELECT count(*) FROM db_agent_definitions WHERE id='$def_id';")
            if [ "${def_landed:-0}" -eq 0 ]; then
                echo "  SKIP  $agent_name (slug conflict in target — definition not imported)"
                ((skipped++)) || true
                continue
            fi

            # Copy system prompt and MCP skills — the agent's behaviour config.
            # Both tables use agent_id FK, so they must follow the def INSERT.
            # Silent no-op when source lacks these tables (pre-v2 DBs).
            sqlite3 "$src_win" \
                "ATTACH '$tgt_win' AS dst;
                 INSERT OR IGNORE INTO dst.db_agent_content (agent_id, content_type, content, updated_at)
                 SELECT agent_id, content_type, content, updated_at
                 FROM db_agent_content WHERE agent_id='$def_id';" 2>/dev/null || true
            sqlite3 "$src_win" \
                "ATTACH '$tgt_win' AS dst;
                 INSERT OR IGNORE INTO dst.db_agent_skills (id, agent_id, name, trigger, skill_type, description, content, created_at)
                 SELECT id, agent_id, name, trigger, skill_type, description, content, created_at
                 FROM db_agent_skills WHERE agent_id='$def_id';" 2>/dev/null || true

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
                   created_at, ${_col_updated_at}, is_seeded, ${_col_user_hidden}
                 FROM db_agent_definitions WHERE id='$def_id';" 2>/dev/null || true

            # Create a stub instance so the agent appears in My Agents
            # (ListRecentSessionsCommand queries instances, not definitions).
            # Derive stub_id from the full def_id (UUID, globally unique) so
            # repeated imports are idempotent and INSERT OR IGNORE is safe.
            # Wire the instance to the agent's existing on-disk Claude session
            # so a single message resumes the real (long) conversation. Falls
            # back to an empty stub when no transcript is found (same behaviour
            # as before this change), so the import never fails on a session-less
            # agent. The slug column is preferred; when it's empty we reproduce
            # the Rust derive_slug() encoding — lower-case AND replace every
            # char outside [a-z0-9_-] with '-' — so names with spaces/dots
            # (e.g. "My Agent" → "my-agent-<id>") still match the on-disk dir.
            local lookup_slug="$agent_slug"
            if [ -z "$lookup_slug" ]; then
                lookup_slug=$(printf '%s' "$agent_name" | tr 'A-Z' 'a-z' | sed 's/[^a-z0-9_-]/-/g')
            fi
            local resolved sess_id="" work_dir=""
            resolved=$(best_session_for_slug "$lookup_slug")
            if [ -n "$resolved" ]; then
                sess_id=$(printf '%s' "$resolved" | cut -f1)
                work_dir=$(printf '%s' "$resolved" | cut -f2)
            fi
            # SQL-escape single quotes (paths/UUIDs rarely contain them, but safe).
            local sess_sql="${sess_id//\'/\'\'}"
            local wd_sql="${work_dir//\'/\'\'}"

            local stub_id="si-$(echo "$def_id" | tr -d '-')"
            sqlite3 "$tgt_win" \
                "INSERT OR IGNORE INTO db_agent_instances
                   (id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, identity_id, memory_id, instance_name,
                    working_directory, display_hidden, started_at, ended_at, created_at)
                 VALUES
                   ('$stub_id', '$def_id', '', '', '$sess_sql', 'stopped',
                    '', '', '', '$safe_name',
                    '$wd_sql', 0, $ts, 0, $ts);"

            # Keep an already-imported stub in sync: earlier runs created the
            # instance with an empty session_id/working_directory, so backfill
            # them now (INSERT OR IGNORE above no-ops on the existing row).
            if [ -n "$sess_sql" ]; then
                sqlite3 "$tgt_win" \
                    "UPDATE db_agent_instances
                        SET session_id='$sess_sql', working_directory='$wd_sql'
                      WHERE id='$stub_id'
                        AND (session_id IS NULL OR session_id='');"
            fi

            if [ -n "$sess_id" ]; then
                echo "        ↳ session $(echo "$sess_id" | cut -c1-8)…  cwd $work_dir"
            fi

            # Mirror instance_name into db_agents so Phase-3b readers
            # (which read db_agents directly) see the agent name immediately,
            # matching what agents_dual_write_instance_create does on the
            # Rust path when a stub is created for a user-clone definition.
            sqlite3 "$tgt_win" \
                "UPDATE db_agents SET instance_name='$safe_name', updated_at=$ts
                 WHERE id='$def_id'
                   AND (instance_name IS NULL OR instance_name='');" 2>/dev/null || true

            # Mirror the resolved working dir into db_agents so a FRESH launch
            # (without resume) also starts in the agent's real directory.
            if [ -n "$wd_sql" ]; then
                sqlite3 "$tgt_win" \
                    "UPDATE db_agents SET working_directory='$wd_sql', updated_at=$ts
                      WHERE id='$def_id'
                        AND (working_directory IS NULL OR working_directory='');" 2>/dev/null || true
            fi

            if [ "$def_exists" -eq 1 ]; then
                echo "  OK    $agent_name (definition already present)"
            else
                echo "  OK    $agent_name"
            fi
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
