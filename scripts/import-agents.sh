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
# For Claude Code that lives at <home>/projects/<slugified-cwd>/<session_id>.jsonl
# where <slugified-cwd> is the agent's working directory with each of : \ . /
# replaced by '-', and <home> is whatever CLAUDE_CONFIG_DIR points at.
#
# IMPORTANT: AgentMux spawns claude with CLAUDE_CONFIG_DIR pointed at its OWN
# isolated home (~/.agentmux/shared/providers/claude), so `claude --resume` reads
# the session from THERE — not from the user's global ~/.claude. Older sessions
# (pre-isolation) live only in the global dir. So we (a) search BOTH roots for
# the best session, and (b) REHYDRATE the chosen session into the isolated home
# at the slug resume expects, so a single message actually replays it.
# (P0 of docs/specs/SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md.)
CLAUDE_PROJECTS="${CLAUDE_PROJECTS:-$HOME/.claude/projects}"
# Where AgentMux's resume actually reads from (CLAUDE_CONFIG_DIR/projects).
SHARED_CLAUDE_PROJECTS="${SHARED_CLAUDE_PROJECTS:-$AGENTMUX_DIR/shared/providers/claude/projects}"
AGENTS_DIR="$AGENTMUX_DIR/agents"

# Slugify a working dir the way Claude Code names its project subdir.
slugify_cwd() { printf '%s' "$1" | sed 's#[:\\./]#-#g'; }

# Resolve the LARGEST Claude session for an agent slug (longest transcript →
# best streaming/perf repro), searching BOTH the isolated AgentMux home and the
# user's global ~/.claude. Echoes "<session_id>\t<working_dir_win>\t<src_jsonl>"
# (tab-separated) or nothing if no session is found.
best_session_for_slug() {
    local slug="$1"
    [ -n "$slug" ] || return 0
    local best_size=0 best_sid="" best_wd="" best_src=""
    local d wd_win cslug jf sz root
    for d in "$AGENTS_DIR/$slug"-*; do
        [ -d "$d" ] || continue
        wd_win=$(win_path "$d")               # C:\Users\...\.agentmux\agents\<name>
        cslug=$(slugify_cwd "$wd_win")
        for root in "$SHARED_CLAUDE_PROJECTS" "$CLAUDE_PROJECTS"; do
            [ -d "$root/$cslug" ] || continue
            while IFS= read -r jf; do
                [ -n "$jf" ] || continue
                # `wc -c` is POSIX; `stat -c%s` is GNU-only and fails silently on
                # BSD/macOS — that would make every size 0 and no-op the wiring.
                sz=$(wc -c < "$jf" 2>/dev/null | tr -d '[:space:]'); sz=${sz:-0}
                if [ "$sz" -gt "$best_size" ]; then
                    best_size=$sz
                    best_sid=$(basename "$jf" .jsonl)
                    best_wd=$wd_win
                    best_src=$jf
                fi
            done < <(find "$root/$cslug" -maxdepth 1 -name '*.jsonl' 2>/dev/null)
        done
    done
    [ -n "$best_sid" ] && printf '%s\t%s\t%s' "$best_sid" "$best_wd" "$best_src"
}

# P0 rehydrate: copy a session transcript into AgentMux's isolated Claude home at
# the slug `claude --resume` expects, so the conversation actually replays even
# when the session only existed in the user's global ~/.claude (pre-isolation).
# No-op when a copy is already present. Echoes a one-line note on success.
rehydrate_claude_session() {
    local sess_id="$1" work_dir="$2" src_jsonl="$3"
    [ -n "$sess_id" ] && [ -f "$src_jsonl" ] || return 0
    local cslug dst_dir dst
    cslug=$(slugify_cwd "$work_dir")
    dst_dir="$SHARED_CLAUDE_PROJECTS/$cslug"
    dst="$dst_dir/$sess_id.jsonl"
    # Already in the isolated home (src == dst, or a copy exists) → nothing to do.
    [ -f "$dst" ] && return 0
    mkdir -p "$dst_dir" 2>/dev/null || return 0
    if cp "$src_jsonl" "$dst" 2>/dev/null; then
        echo "        ↳ rehydrated session into shared/providers/claude (resume will find it)"
    fi
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

        local src_win; src_win=$(win_path "$src_db")

        # Resilience: one unreadable / locked / corrupt / table-less source DB
        # must never abort the whole import. Under `set -euo pipefail` the first
        # sqlite error would propagate and silently drop every agent in DBs not
        # yet scanned (observed: a stale `*.bak` channel mid-scan dropped a real
        # agent whose data lived in a later-sorted DB). Probe the source first.
        # Old pre-agent-schema versions (no db_agent_definitions table) are
        # expected and numerous — skip those silently; WARN only on genuine
        # problems (corruption, locks).
        local _probe_err
        if ! _probe_err=$(sqlite3 "$src_win" \
            "SELECT 1 FROM db_agent_definitions LIMIT 1;" 2>&1 >/dev/null); then
            case "$_probe_err" in
                *"no such table"*) : ;;  # pre-agent-schema DB — nothing to import
                *) echo "  WARN  skipping unreadable source DB (${_probe_err%%$'\n'*}): $src_db" >&2 ;;
            esac
            continue
        fi

        # Detect which newer columns exist in the source schema so the SELECTs
        # below never reference a missing column. This MUST run BEFORE the agents
        # query: referencing an absent column (e.g. `slug` on a very old DB) makes
        # sqlite3 error, and under `set -euo pipefail` the failing
        # command-substitution assignment aborts the entire import silently
        # (db_query swallows stderr with its own 2>/dev/null).
        local _src_cols; _src_cols=$(sqlite3 "$src_win" \
            "SELECT group_concat(name) FROM pragma_table_info('db_agent_definitions');" \
            2>/dev/null || echo "")
        local _col_updated_at _col_user_hidden _col_slug
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
        if echo "$_src_cols" | grep -qw "slug"; then
            _col_slug="slug"
        else
            _col_slug="''"
        fi

        # Use char(1) (ASCII unit-separator) instead of '|' so agent names that
        # contain a pipe character don't corrupt the field split.
        # sqlite3.exe emits CRLF on Windows; the trailing CR contaminates the
        # last char(1)-delimited field (the slug), which silently breaks the
        # on-disk session lookup below. Strip CR so every field is clean.
        # Distinguish a genuine query FAILURE (corruption hit mid-scan, or a
        # column the WHERE/SELECT names — e.g. is_seeded — missing despite the
        # table existing, which the up-front probe doesn't catch) from an empty
        # result. A bare `|| agents=""` would mask the failure as "no agents"
        # and skip the DB with no WARN (codex P2 on #1380). Capture stdout+stderr
        # with the exit code; on failure WARN + skip this source DB.
        local _agents_out
        if _agents_out=$(sqlite3 "$src_win" \
            "SELECT id||char(1)||name||char(1)||COALESCE(${_col_slug},'') FROM db_agent_definitions WHERE $where;" 2>&1); then
            agents=$(printf '%s' "$_agents_out" | tr -d '\r')
        else
            echo "  WARN  skipping source DB (agent query failed: ${_agents_out%%$'\n'*}): $src_db" >&2
            continue
        fi
        [ -z "$agents" ] && continue

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
            # A schema mismatch in this single source DB (e.g. an old or `*.bak`
            # DB missing a column named below) must skip THIS agent, not abort
            # the whole import. Capture stderr and continue on failure rather
            # than letting `set -e` propagate the error and drop later agents.
            local _def_copy_err
            if ! _def_copy_err=$(sqlite3 "$src_win" \
                "ATTACH '$tgt_win' AS dst;
                 INSERT OR IGNORE INTO dst.db_agent_definitions
                   (id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start, restart_on_crash,
                    idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id,
                    is_seeded, accounts, parent_id, branch_label, updated_at, user_hidden)
                 SELECT
                    id, ${_col_slug}, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start, restart_on_crash,
                    idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id,
                    is_seeded, accounts, parent_id, branch_label,
                    ${_col_updated_at}, ${_col_user_hidden}
                 FROM db_agent_definitions WHERE id='$def_id';" 2>&1); then
                case "$_def_copy_err" in
                    *"dst."*)
                        # Error names the attached TARGET schema (dst.*): the
                        # target table is missing/corrupt or the target DB is not
                        # an AgentMux DB. That's a target-side failure — abort,
                        # don't mask it as a per-agent source skip (codex P2 on
                        # #1380: "no such table: dst.db_agent_definitions").
                        echo "ERROR: import aborted — target DB unusable for '$agent_name': ${_def_copy_err%%$'\n'*}" >&2
                        exit 1
                        ;;
                    *"no such column"*|*"no such table"*)
                        # SOURCE schema mismatch (an old / *.bak source missing a
                        # column the SELECT names; the up-front probe already
                        # proved the source TABLE exists) — skip THIS agent.
                        echo "  WARN  $agent_name — source schema mismatch; skipping (${_def_copy_err%%$'\n'*})" >&2
                        ((skipped++)) || true
                        continue
                        ;;
                    *)
                        # Any other failure (locked, read-only, full disk, I/O) is
                        # target-side too — abort. Silently skipping every agent
                        # and exiting 0 would mask that the import never happened
                        # (codex P2 on #1380).
                        echo "ERROR: import aborted — write failed for '$agent_name': ${_def_copy_err%%$'\n'*}" >&2
                        exit 1
                        ;;
                esac
            fi

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
                   ${_col_slug}, branch_label,
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
            local resolved sess_id="" work_dir="" src_jsonl=""
            # best_session_for_slug returns non-zero when the agent has no
            # on-disk session (a normal case); `|| resolved=""` keeps that from
            # aborting the whole import under `set -e`.
            resolved=$(best_session_for_slug "$lookup_slug") || resolved=""
            if [ -n "$resolved" ]; then
                # Tab-delimited fields; filesystem paths never contain a literal
                # tab, so the split is unambiguous.
                IFS=$'\t' read -r sess_id work_dir src_jsonl <<< "$resolved"
                # P0: ensure the chosen session lives in AgentMux's isolated Claude
                # home, which is where `claude --resume` reads — otherwise the pane
                # stays empty even though session_id is wired.
                rehydrate_claude_session "$sess_id" "$work_dir" "$src_jsonl"
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
    # Use an `if` (not `&&`) so a zero-import run doesn't make this function —
    # and the whole script under `set -e` — exit non-zero.
    if [ "$imported" -gt 0 ]; then
        echo "Reopen the agent pane in the $target_version build to see the changes."
    fi
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
