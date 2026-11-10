// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Several Kanban boards per person (docs/specs/SPEC_SHARED_KANBAN_ACROSS_MACHINES_2026_11_01.md §9).
//!
//! - `db_boards`: one row per board: its slug, name, which agents see it, whether it is archived or
//!   the default, and its own task counter.
//! - `db_work_cards.board_id` and `board_num`: which board a task is on and its number there. The
//!   card's `num` stays the store's own key; what people and agents say (`#168`, `work#12`) is
//!   `board_num`, counted per board.
//!
//! Today's board becomes "Main" (id `main`, the default) the first time this schema runs, and every
//! card keeps its number. A bare `#n` is always the default board's.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::work_queue::{insert_item, row_to_item, work_state, WorkItem, COLS};
use super::StoreError;

/// The id of the board every existing task is on after the first migration.
pub const MAIN_ID: &str = "main";
/// The longest slug.
pub const MAX_SLUG: usize = 24;
/// The longest display name.
pub const MAX_NAME: usize = 60;
/// How long a changed slug keeps working as an alias.
pub const ALIAS_MS: i64 = 90 * 24 * 60 * 60 * 1000;
/// The visible countdown, with a veto, before a board with tasks is deleted.
pub const DELETE_COUNTDOWN_MS: i64 = 15_000;
/// What `board: "all"` names, so no board may take it as a slug.
pub const ALL: &str = "all";

/// The tables, created with the board's (`work_board::SCHEMA`) by [`apply_schema`].
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_boards (
        id                TEXT PRIMARY KEY,
        slug              TEXT NOT NULL UNIQUE,
        name              TEXT NOT NULL,
        tier              TEXT NOT NULL DEFAULT 'local',
        readable          INTEGER NOT NULL DEFAULT 1,
        agents            TEXT NOT NULL DEFAULT '',
        is_default        INTEGER NOT NULL DEFAULT 0,
        archived          INTEGER NOT NULL DEFAULT 0,
        next_num          INTEGER NOT NULL DEFAULT 1,
        aliases           TEXT NOT NULL DEFAULT '',
        pending_delete_at INTEGER,
        created_by        TEXT NOT NULL DEFAULT '',
        created_at        INTEGER NOT NULL DEFAULT 0,
        updated_at        INTEGER NOT NULL DEFAULT 0
    );";

/// Columns added to a cards table that older builds created.
const CARD_COLUMNS: [&str; 2] = [
    "ALTER TABLE db_work_cards ADD COLUMN board_id TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE db_work_cards ADD COLUMN board_num INTEGER NOT NULL DEFAULT 0",
];

/// Create the Kanban's tables, add the board columns, and (once) turn today's board into "Main".
/// Run on every open of the identity store, with no schema version bump (see `work_claims::SCHEMA`).
pub(crate) fn apply_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(super::work_board::SCHEMA)?;
    conn.execute_batch(SCHEMA)?;
    for stmt in CARD_COLUMNS {
        if let Err(e) = conn.execute_batch(stmt) {
            if !e.to_string().contains("duplicate column") {
                return Err(e.into());
            }
        }
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_work_cards_board_num
             ON db_work_cards(board_id, board_num) WHERE board_id <> '';
         CREATE INDEX IF NOT EXISTS idx_ids_work_cards_board ON db_work_cards(board_id);",
    )?;
    // Seeding Main is what marks the migration done: only then does every existing card keep its
    // number. A card an older build adds later is numbered from Main's counter instead
    // (`number_unboarded`), so it can't collide with a number handed out since.
    // Only into an empty table: Main may have been deleted since, and must not come back as a
    // second default beside the board the person chose.
    let seeded = conn.execute(
        "INSERT OR IGNORE INTO db_boards (id, slug, name, is_default, created_at, updated_at)
         SELECT ?1, 'main', 'Main', 1, 0, 0 WHERE NOT EXISTS (SELECT 1 FROM db_boards)",
        params![MAIN_ID],
    )?;
    if seeded > 0 {
        conn.execute("UPDATE db_work_cards SET board_id = ?1, board_num = num WHERE board_id = ''", params![MAIN_ID])?;
        conn.execute(
            "UPDATE db_boards SET next_num = (SELECT COALESCE(MAX(board_num), 0) + 1 FROM db_work_cards WHERE board_id = ?1)
              WHERE id = ?1",
            params![MAIN_ID],
        )?;
    }
    Ok(())
}

/// Why a board action was refused (`Refused` is worded for the person or agent who asked).
#[derive(Debug)]
pub enum BoardError {
    Refused(String),
    Store(StoreError),
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardError::Refused(why) => f.write_str(why),
            BoardError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl From<StoreError> for BoardError {
    fn from(e: StoreError) -> Self {
        BoardError::Store(e)
    }
}

impl From<rusqlite::Error> for BoardError {
    fn from(e: rusqlite::Error) -> Self {
        BoardError::Store(e.into())
    }
}

fn refused<T>(why: impl Into<String>) -> Result<T, BoardError> {
    Err(BoardError::Refused(why.into()))
}

/// A slug a board used to have, still accepted until `until` (ms).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlugAlias {
    pub slug: String,
    pub until: i64,
}

/// One board.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub id: String,
    /// Lowercase letters, digits and hyphen; what a ref says (`work#12`).
    pub slug: String,
    pub name: String,
    /// `local` (this machine). `lan` and `cloud` arrive with the sync layer.
    pub tier: String,
    /// For a cloud board: whether AgentMux Cloud may read it.
    pub readable: bool,
    /// The agents (ids or UIDs) that see it; empty means all.
    pub agents: Vec<String>,
    pub is_default: bool,
    pub archived: bool,
    pub aliases: Vec<SlugAlias>,
    /// Set while a delete waits out its countdown: when it happens (ms).
    pub pending_delete_at: Option<i64>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Tasks not finished, and all tasks, on this board (filled by [`Store::work_boards_list`]).
    #[serde(default)]
    pub open: i64,
    #[serde(default)]
    pub total: i64,
}

impl Board {
    /// Whether an agent sees this board: no list means all agents.
    pub fn visible_to(&self, agent_id: &str, agent_uid: &str) -> bool {
        self.agents.is_empty()
            || self.agents.iter().any(|a| a == agent_id)
            || (!agent_uid.is_empty() && self.agents.iter().any(|a| a == agent_uid))
    }

    /// How a task on this board is written: `#12` on the default board, else `work#12`.
    pub fn task_ref(&self, num: i64) -> String {
        if self.is_default {
            format!("#{num}")
        } else {
            format!("{}#{num}", self.slug)
        }
    }
}

const BOARD_COLS: &str = "id, slug, name, tier, readable, agents, is_default, archived, aliases, pending_delete_at, \
                          created_by, created_at, updated_at";

fn row_to_board(r: &rusqlite::Row) -> rusqlite::Result<Board> {
    let agents: String = r.get("agents")?;
    let aliases: String = r.get("aliases")?;
    Ok(Board {
        id: r.get("id")?,
        slug: r.get("slug")?,
        name: r.get("name")?,
        tier: r.get("tier")?,
        readable: r.get::<_, i64>("readable")? != 0,
        agents: serde_json::from_str(&agents).unwrap_or_default(),
        is_default: r.get::<_, i64>("is_default")? != 0,
        archived: r.get::<_, i64>("archived")? != 0,
        aliases: serde_json::from_str(&aliases).unwrap_or_default(),
        pending_delete_at: r.get("pending_delete_at")?,
        created_by: r.get("created_by")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        open: 0,
        total: 0,
    })
}

/// What a task ref says: its board's slug (none for a bare number) and its number.
pub fn parse_ref(text: &str) -> Option<(Option<String>, i64)> {
    let t = text.trim();
    let (slug, num) = match t.split_once('#') {
        Some((slug, num)) => (slug.trim(), num),
        None => ("", t),
    };
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let num = num.parse().ok()?;
    if slug.is_empty() {
        return Some((None, num));
    }
    let slug = slug.to_ascii_lowercase();
    valid_slug(&slug).then_some((Some(slug), num))
}

/// Lowercase letters, digits and hyphen, up to [`MAX_SLUG`], not starting with a hyphen.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= MAX_SLUG
        && !slug.starts_with('-')
        && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A slug from a name: `Client X` is `client-x`.
fn slug_of(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(MAX_SLUG).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "board".into() } else { out }
}

fn clean_name(name: &str) -> Result<String, BoardError> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return refused("a board needs a name");
    }
    if name.chars().count() > MAX_NAME {
        return refused(format!("a board's name is at most {MAX_NAME} characters"));
    }
    Ok(name)
}

fn agents_json(agents: &[String]) -> String {
    let mut list: Vec<&str> = agents.iter().map(|a| a.trim()).filter(|a| !a.is_empty()).collect();
    list.sort_unstable();
    list.dedup();
    if list.is_empty() { String::new() } else { serde_json::to_string(&list).unwrap_or_default() }
}

/// The board named by `key`: its id, its slug, an alias still in force, or its display name
/// (any case).
fn find(conn: &Connection, key: &str, now_ms: i64) -> Result<Option<Board>, StoreError> {
    let key = key.trim();
    if key.is_empty() {
        return Ok(None);
    }
    let sql = format!("SELECT {BOARD_COLS} FROM db_boards ORDER BY is_default DESC, created_at ASC, slug ASC");
    let mut stmt = conn.prepare(&sql)?;
    let boards = stmt.query_map([], row_to_board)?.collect::<Result<Vec<_>, _>>()?;
    let lower = key.to_lowercase();
    let pick = |f: &dyn Fn(&Board) -> bool| boards.iter().find(|b| f(b)).cloned();
    Ok(pick(&|b| b.id == key)
        .or_else(|| pick(&|b| b.slug == lower))
        .or_else(|| pick(&|b| b.aliases.iter().any(|a| a.slug == lower && a.until > now_ms)))
        .or_else(|| pick(&|b| b.name.to_lowercase() == lower)))
}

fn default_board(conn: &Connection) -> Result<Board, StoreError> {
    let sql = format!("SELECT {BOARD_COLS} FROM db_boards WHERE is_default = 1 LIMIT 1");
    match conn.query_row(&sql, [], row_to_board).optional()? {
        Some(b) => Ok(b),
        None => {
            // The default is only ever moved, never cleared; if a hand edit left none, the oldest board is it
            // (Main, unless it was deleted).
            conn.execute(
                "UPDATE db_boards SET is_default = 1 WHERE id = (SELECT id FROM db_boards ORDER BY created_at, id LIMIT 1)",
                [],
            )?;
            Ok(conn.query_row(&sql, [], row_to_board)?)
        }
    }
}

pub(crate) fn default_board_id(conn: &Connection) -> Result<String, StoreError> {
    Ok(default_board(conn)?.id)
}

/// The wall clock in ms, for the lookups that have no `now` of their own (an alias in force).
pub(crate) fn now_ms_clock() -> i64 {
    agentmux_common::time::now_ms()
}

/// Give each task its board's slug and the way its ref is written.
pub(crate) fn attach_boards(conn: &Connection, tasks: &mut [super::work_board::BoardTask]) -> Result<(), StoreError> {
    if tasks.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare("SELECT id, slug, is_default FROM db_boards")?;
    let boards: std::collections::HashMap<String, (String, bool)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0))))?
        .collect::<Result<_, _>>()?;
    // A card with no board yet is on the default one.
    let fallback = default_board(conn)?;
    for t in tasks {
        let (slug, default) = boards.get(&t.card.board_id).cloned().unwrap_or_else(|| (fallback.slug.clone(), true));
        t.card.task_ref = if default { format!("#{}", t.card.num) } else { format!("{slug}#{}", t.card.num) };
        t.card.board = slug;
    }
    Ok(())
}

/// The next number on a board, counted from 1 and never reused.
pub(crate) fn alloc_num(conn: &Connection, board_id: &str) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "UPDATE db_boards SET next_num = next_num + 1 WHERE id = ?1 RETURNING next_num - 1",
        params![board_id],
        |r| r.get(0),
    )?)
}

/// Put cards an older build added (no board yet) on Main, numbered from its counter.
pub(crate) fn number_unboarded(conn: &Connection) -> Result<(), StoreError> {
    let mut stmt = conn.prepare("SELECT num FROM db_work_cards WHERE board_id = '' ORDER BY num")?;
    let nums = stmt.query_map([], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?;
    if nums.is_empty() {
        return Ok(());
    }
    // Main's counter, unless Main was deleted: then the default board's.
    let board = default_board_id(conn)?;
    for num in nums {
        let n = alloc_num(conn, &board)?;
        conn.execute("UPDATE db_work_cards SET board_id = ?1, board_num = ?2 WHERE num = ?3", params![board, n, num])?;
    }
    Ok(())
}

/// Resolve the board a call names: none means the default board.
pub(crate) fn resolve(conn: &Connection, key: Option<&str>, now_ms: i64) -> Result<Board, BoardError> {
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        None => Ok(default_board(conn)?),
        Some(k) => match find(conn, k, now_ms)? {
            Some(b) => Ok(b),
            None => refused(format!("no board named \"{k}\"")),
        },
    }
}

/// What a board create or edit sets. `None` leaves a field as it is.
#[derive(Debug, Clone, Default)]
pub struct BoardEdit {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub tier: Option<String>,
    pub readable: Option<bool>,
    /// The agents that see it; an empty list means all.
    pub agents: Option<Vec<String>>,
}

/// What a move did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskMoved {
    pub id: String,
    pub from_ref: String,
    pub to_ref: String,
    pub board: String,
    pub num: i64,
    /// Tasks that no longer start after it, or that it no longer starts after.
    pub dropped_deps: Vec<String>,
    /// Its target agent, cleared because the board's agent list does not include it.
    pub target_cleared: bool,
}

/// Where a delete stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Deletion {
    Deleted,
    /// A board with tasks waits out a visible countdown, with a veto; it goes at this time (ms).
    Pending(i64),
}

fn check_tier(tier: &str) -> Result<String, BoardError> {
    match tier.trim() {
        "" | "local" => Ok("local".into()),
        "lan" | "cloud" => refused("sharing a board across machines arrives with the sync layer; for now a board stays on this machine"),
        other => refused(format!("tier is \"local\" for now, not \"{other}\"")),
    }
}

impl Store {
    /// The boards, default first, with how many tasks each holds. Archived ones only when asked.
    pub fn work_boards_list(&self, include_archived: bool, now_ms: i64) -> Result<Vec<Board>, StoreError> {
        let _ = self.work_boards_finish_deletes(now_ms);
        let conn = self.conn().lock().unwrap();
        number_unboarded(&conn)?;
        let sql = format!(
            "SELECT {BOARD_COLS},
                    (SELECT COUNT(*) FROM db_work_cards c JOIN db_work_queue q ON q.id = c.work_id
                      WHERE c.board_id = b.id AND q.state NOT IN ('{done}', '{cancelled}', '{failed}')) AS open_n,
                    (SELECT COUNT(*) FROM db_work_cards c WHERE c.board_id = b.id) AS total_n
               FROM db_boards b
              WHERE (?1 OR b.archived = 0)
              ORDER BY b.is_default DESC, b.created_at ASC, b.slug ASC",
            done = work_state::DONE,
            cancelled = work_state::CANCELLED,
            failed = work_state::FAILED,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![include_archived], |r| {
            let mut b = row_to_board(r)?;
            b.open = r.get("open_n")?;
            b.total = r.get("total_n")?;
            Ok(b)
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Tasks in `state` (all when empty) on the boards `board_ids`, newest-updated first.
    pub fn work_queue_list_on(&self, state: &str, limit: usize, board_ids: &[String], now_ms: i64) -> Result<Vec<WorkItem>, StoreError> {
        if board_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn().lock().unwrap();
        number_unboarded(&conn)?;
        let mut stmt = conn.prepare("SELECT id FROM db_work_queue WHERE id NOT IN (SELECT work_id FROM db_work_cards)")?;
        let missing = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        for id in &missing {
            super::work_board::ensure_card(&conn, id, now_ms)?;
        }
        let marks: Vec<String> = (0..board_ids.len()).map(|i| format!("?{}", i + 3)).collect();
        let sql = format!(
            "SELECT {q} FROM db_work_queue q JOIN db_work_cards c ON c.work_id = q.id
              WHERE (?1 = '' OR q.state = ?1) AND c.board_id IN ({marks})
              ORDER BY q.updated_at DESC, q.created_at DESC
              LIMIT ?2",
            q = super::work_board::q_cols(),
            marks = marks.join(","),
        );
        let limit = limit as i64;
        let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&state, &limit];
        for id in board_ids {
            binds.push(id);
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(binds.as_slice(), row_to_item)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Open tasks aimed at this agent on boards other than `except`, which it sees: the board names
    /// with how many each (for the line "3 more for you on Work and Client X").
    pub fn work_waiting_elsewhere(&self, except: &str, agent_id: &str, agent_uid: &str) -> Result<Vec<(String, i64)>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT b.name, COUNT(*)
               FROM db_work_queue q JOIN db_work_cards c ON c.work_id = q.id JOIN db_boards b ON b.id = c.board_id
              WHERE q.state = 'open' AND b.archived = 0 AND b.id <> ?1
                AND ((q.target_agent_uid <> '' AND q.target_agent_uid = ?3)
                     OR (q.target_agent_uid = '' AND q.target_agent <> '' AND q.target_agent = ?2))
                AND (b.agents = '' OR instr(b.agents, '\"' || ?2 || '\"') > 0
                     OR (?3 <> '' AND instr(b.agents, '\"' || ?3 || '\"') > 0))
              GROUP BY b.id ORDER BY b.created_at, b.slug",
        )?;
        let rows = stmt.query_map(params![except, agent_id, agent_uid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The board named by `key` (id, slug, an alias in force, or display name), or the default
    /// board when none is named.
    pub fn work_board_resolve(&self, key: Option<&str>, now_ms: i64) -> Result<Board, BoardError> {
        let conn = self.conn().lock().unwrap();
        resolve(&conn, key, now_ms)
    }

    /// The default board.
    pub fn work_board_default(&self) -> Result<Board, StoreError> {
        let conn = self.conn().lock().unwrap();
        default_board(&conn)
    }

    /// The board a task is on and its number there.
    pub fn work_task_board(&self, work_id: &str) -> Result<Option<(Board, i64)>, StoreError> {
        let conn = self.conn().lock().unwrap();
        number_unboarded(&conn)?;
        let Some((board_id, num)) = conn
            .query_row("SELECT board_id, board_num FROM db_work_cards WHERE work_id = ?1", params![work_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .optional()?
        else {
            return Ok(None);
        };
        Ok(find(&conn, &board_id, 0)?.map(|b| (b, num)))
    }

    /// Make a board. The slug comes from the name unless one is given (and then it must be free).
    pub fn work_board_create(&self, edit: &BoardEdit, by: &str, now_ms: i64) -> Result<Board, BoardError> {
        let name = clean_name(edit.name.as_deref().unwrap_or(""))?;
        let tier = check_tier(edit.tier.as_deref().unwrap_or(""))?;
        let conn = self.conn().lock().unwrap();
        let slug = match edit.slug.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => {
                let s = s.to_ascii_lowercase();
                slug_free(&conn, &s, "", now_ms)?;
                s
            }
            None => {
                let base = slug_of(&name);
                let mut candidate = base.clone();
                let mut n = 1;
                while slug_free(&conn, &candidate, "", now_ms).is_err() {
                    n += 1;
                    let tail = format!("-{n}");
                    candidate = format!("{}{tail}", base.chars().take(MAX_SLUG - tail.len()).collect::<String>());
                }
                candidate
            }
        };
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO db_boards (id, slug, name, tier, readable, agents, created_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![id, slug, name, tier, edit.readable.unwrap_or(true), agents_json(edit.agents.as_deref().unwrap_or(&[])), by, now_ms],
        )?;
        Ok(find(&conn, &id, now_ms)?.ok_or(StoreError::NotFound)?)
    }

    /// Change a board's name, slug, sharing, readable switch or agent list. A new slug keeps the
    /// old one as an alias for [`ALIAS_MS`].
    pub fn work_board_edit(&self, key: &str, edit: &BoardEdit, now_ms: i64) -> Result<Board, BoardError> {
        let conn = self.conn().lock().unwrap();
        let board = resolve(&conn, Some(key), now_ms)?;
        let name = match &edit.name {
            Some(n) => clean_name(n)?,
            None => board.name.clone(),
        };
        let tier = match &edit.tier {
            Some(t) => check_tier(t)?,
            None => board.tier.clone(),
        };
        let mut slug = board.slug.clone();
        let mut aliases: Vec<SlugAlias> = board.aliases.iter().filter(|a| a.until > now_ms).cloned().collect();
        if let Some(new) = edit.slug.as_deref().map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty() && *s != board.slug) {
            if board.id == MAIN_ID {
                return refused("the Main board keeps the slug \"main\", so bare refs like #168 keep their meaning");
            }
            slug_free(&conn, &new, &board.id, now_ms)?;
            aliases.retain(|a| a.slug != new);
            aliases.push(SlugAlias { slug: board.slug.clone(), until: now_ms + ALIAS_MS });
            slug = new;
        }
        let readable = edit.readable.unwrap_or(board.readable);
        let agents = match &edit.agents {
            Some(a) => agents_json(a),
            None => agents_json(&board.agents),
        };
        conn.execute(
            "UPDATE db_boards SET name = ?2, slug = ?3, tier = ?4, readable = ?5, agents = ?6, aliases = ?7, updated_at = ?8
              WHERE id = ?1",
            params![board.id, name, slug, tier, readable, agents, serde_json::to_string(&aliases).unwrap_or_default(), now_ms],
        )?;
        Ok(find(&conn, &board.id, now_ms)?.ok_or(StoreError::NotFound)?)
    }

    /// Hide a board from the picker's main list and make it read-only: no new tasks, no claims.
    pub fn work_board_archive(&self, key: &str, archived: bool, now_ms: i64) -> Result<Board, BoardError> {
        let conn = self.conn().lock().unwrap();
        let board = resolve(&conn, Some(key), now_ms)?;
        if archived && board.is_default {
            return refused("the default board can't be archived; make another board the default first");
        }
        conn.execute(
            "UPDATE db_boards SET archived = ?2, pending_delete_at = CASE WHEN ?2 = 0 THEN NULL ELSE pending_delete_at END,
                    updated_at = ?3 WHERE id = ?1",
            params![board.id, archived, now_ms],
        )?;
        Ok(find(&conn, &board.id, now_ms)?.ok_or(StoreError::NotFound)?)
    }

    /// Make a board the default: the one a call that names none means, and the one a bare `#n` is on.
    pub fn work_board_set_default(&self, key: &str, now_ms: i64) -> Result<Board, BoardError> {
        let conn = self.conn().lock().unwrap();
        let board = resolve(&conn, Some(key), now_ms)?;
        if board.archived {
            return refused("an archived board can't be the default; unarchive it first");
        }
        conn.execute("UPDATE db_boards SET is_default = (id = ?1), updated_at = ?2 WHERE is_default = 1 OR id = ?1", params![board.id, now_ms])?;
        Ok(find(&conn, &board.id, now_ms)?.ok_or(StoreError::NotFound)?)
    }

    /// Delete an archived board and its tasks. A board with tasks waits [`DELETE_COUNTDOWN_MS`]
    /// first, so the person can veto ([`Self::work_board_cancel_delete`]); one with none goes now.
    pub fn work_board_delete(&self, key: &str, now_ms: i64) -> Result<Deletion, BoardError> {
        let conn = self.conn().lock().unwrap();
        let board = resolve(&conn, Some(key), now_ms)?;
        if board.is_default {
            return refused("the default board can't be deleted");
        }
        if !board.archived {
            return refused("only an archived board can be deleted; archive it first");
        }
        let tasks: i64 = conn.query_row("SELECT COUNT(*) FROM db_work_cards WHERE board_id = ?1", params![board.id], |r| r.get(0))?;
        if tasks == 0 {
            conn.execute("DELETE FROM db_boards WHERE id = ?1", params![board.id])?;
            return Ok(Deletion::Deleted);
        }
        let at = now_ms + DELETE_COUNTDOWN_MS;
        conn.execute("UPDATE db_boards SET pending_delete_at = ?2, updated_at = ?3 WHERE id = ?1", params![board.id, at, now_ms])?;
        Ok(Deletion::Pending(at))
    }

    /// Veto a delete that is counting down.
    pub fn work_board_cancel_delete(&self, key: &str, now_ms: i64) -> Result<Board, BoardError> {
        let conn = self.conn().lock().unwrap();
        let board = resolve(&conn, Some(key), now_ms)?;
        conn.execute("UPDATE db_boards SET pending_delete_at = NULL, updated_at = ?2 WHERE id = ?1", params![board.id, now_ms])?;
        Ok(find(&conn, &board.id, now_ms)?.ok_or(StoreError::NotFound)?)
    }

    /// Carry out the deletes whose countdown is over. Returns the slugs of the boards that went.
    pub fn work_boards_finish_deletes(&self, now_ms: i64) -> Result<Vec<String>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, slug FROM db_boards WHERE pending_delete_at IS NOT NULL AND pending_delete_at <= ?1 AND archived = 1 AND is_default = 0",
        )?;
        let due = stmt.query_map(params![now_ms], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        let mut gone = Vec::new();
        for (id, slug) in due {
            let tx = conn.unchecked_transaction()?;
            let mut stmt = tx.prepare("SELECT work_id FROM db_work_cards WHERE board_id = ?1")?;
            let ids = stmt.query_map(params![id], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            for work_id in &ids {
                purge_task(&tx, work_id)?;
            }
            tx.execute("DELETE FROM db_boards WHERE id = ?1", params![id])?;
            tx.commit()?;
            gone.push(slug);
        }
        Ok(gone)
    }

    /// Move a task to another board: a new task there with a number from that board's counter, the
    /// same title, payload, links and media, and a "moved" line at the top of its history. The
    /// original closes as done, with "Moved to ..." as its result, and keeps its history.
    ///
    /// A claimed task, or one in review, can't move. One with `after` edges (either way) can't
    /// either unless `drop_deps`, which removes the edges.
    pub fn work_task_move(&self, task: &str, to: &str, actor: &str, drop_deps: bool, now_ms: i64) -> Result<TaskMoved, BoardError> {
        let conn = self.conn().lock().unwrap();
        number_unboarded(&conn)?;
        let target = resolve(&conn, Some(to), now_ms)?;
        if target.archived {
            return refused(format!("board {} is archived; unarchive it first", target.slug));
        }
        let (source_board, source_num, item) = task_to_move(&conn, task, now_ms)?;
        if source_board.id == target.id {
            return refused(format!("it is already on board {}", target.slug));
        }
        let from_ref = source_board.task_ref(source_num);
        match item.state.as_str() {
            s if s == work_state::OPEN || s == work_state::HELD => {}
            s if s == work_state::DONE || s == work_state::CANCELLED || s == work_state::FAILED => {
                return refused(format!("{from_ref} is finished; only a task that has not started can move"));
            }
            _ => return refused(format!("{from_ref} is {}; release it or finish it first, then move it", item.state)),
        }
        let edges = edges_of(&conn, &item.id)?;
        if !edges.is_empty() && !drop_deps {
            return refused(format!(
                "{from_ref} has dependencies ({}); they stay inside one board. Move it with drop_deps to remove them",
                edges.join(", ")
            ));
        }

        let tx = conn.unchecked_transaction()?;
        // Another AgentMux sharing the store may have claimed it since it was checked: close it only if it
        // is still as it was read, as the first write, so the whole move rolls back if it is not.
        let closed = tx.execute(
            "UPDATE db_work_queue SET state = ?2, updated_at = ?3 WHERE id = ?1 AND state = ?4",
            params![item.id, work_state::DONE, now_ms, item.state],
        )?;
        if closed != 1 {
            return refused(format!("{from_ref} changed while it was being moved; look at it again"));
        }
        tx.execute("DELETE FROM db_work_deps WHERE work_id = ?1 OR after_id = ?1", params![item.id])?;
        let target_cleared = (!item.target_agent.is_empty() || !item.target_agent_uid.is_empty())
            && !target.visible_to(&item.target_agent, &item.target_agent_uid);
        let new_id = uuid::Uuid::new_v4().to_string();
        let mut moved_item = WorkItem { id: new_id.clone(), updated_at: now_ms, ..item.clone() };
        if target_cleared {
            moved_item.target_agent.clear();
            moved_item.target_agent_uid.clear();
        }
        insert_item(&tx, &moved_item)?;
        let num = alloc_num(&tx, &target.id)?;
        tx.execute(
            "INSERT INTO db_work_cards
                (work_id, repo, branch, pr_url, rank, review_required, checklist, question, note, requested_by,
                 requested_by_kind, request_text, filed_by, updated_at, board_id, board_num)
             SELECT ?2, repo, branch, pr_url, rank, review_required, checklist, '', note, requested_by,
                    requested_by_kind, request_text, filed_by, ?3, ?4, ?5
               FROM db_work_cards WHERE work_id = ?1",
            params![item.id, new_id, now_ms, target.id, num],
        )?;
        tx.execute("INSERT OR IGNORE INTO db_work_docs (work_id, path, updated_at) SELECT ?2, path, ?3 FROM db_work_docs WHERE work_id = ?1", params![item.id, new_id, now_ms])?;
        tx.execute(
            "INSERT OR IGNORE INTO db_work_ui (work_id, ui_change, updated_at) SELECT ?2, ui_change, ?3 FROM db_work_ui WHERE work_id = ?1",
            params![item.id, new_id, now_ms],
        )?;
        tx.execute("UPDATE db_work_media SET work_id = ?2 WHERE work_id = ?1", params![item.id, new_id])?;
        let to_ref = target.task_ref(num);
        let (opened, closed) = (format!("moved from {from_ref} by {actor}"), format!("moved to {to_ref} by {actor}"));
        for (work_id, kind, from, to, body) in [
            (&new_id, "moved", "", moved_item.state.as_str(), opened.as_str()),
            (&item.id, "moved", item.state.as_str(), work_state::DONE, closed.as_str()),
        ] {
            tx.execute(
                "INSERT INTO db_work_events (work_id, at, actor, actor_kind, kind, from_state, to_state, body)
                 VALUES (?1, ?2, ?3, 'agent', ?4, ?5, ?6, ?7)",
                params![work_id, now_ms, actor, kind, from, to, body],
            )?;
        }
        tx.execute(
            "UPDATE db_work_queue SET state = ?2, result = ?3, updated_at = ?4 WHERE id = ?1",
            params![item.id, work_state::DONE, format!("Moved to {to_ref}"), now_ms],
        )?;
        tx.execute("UPDATE db_work_cards SET updated_at = ?2 WHERE work_id = ?1", params![item.id, now_ms])?;
        tx.commit()?;
        Ok(TaskMoved { id: new_id, from_ref, to_ref, board: target.slug, num, dropped_deps: edges, target_cleared })
    }
}

/// The task a move names (an id or a `slug#n` ref), with the board it is on and its number there.
fn task_to_move(conn: &Connection, task: &str, now_ms: i64) -> Result<(Board, i64, WorkItem), BoardError> {
    let key = task.trim();
    let found = match parse_ref(key) {
        Some((slug, n)) => {
            let b = resolve(conn, slug.as_deref(), now_ms)?;
            conn.query_row(
                "SELECT work_id FROM db_work_cards WHERE board_id = ?1 AND board_num = ?2",
                params![b.id, n],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        }
        None => Some(key.to_string()),
    };
    let Some(work_id) = found else { return refused(format!("no task {key}")) };
    let Some(item) = conn
        .query_row(&format!("SELECT {COLS} FROM db_work_queue WHERE id = ?1"), params![work_id], row_to_item)
        .optional()?
    else {
        return refused(format!("no task {key}"));
    };
    let (board_id, num): (String, i64) = conn.query_row(
        "SELECT board_id, board_num FROM db_work_cards WHERE work_id = ?1",
        params![item.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((find(conn, &board_id, 0)?.ok_or(StoreError::NotFound)?, num, item))
}

/// The refs of the tasks a task starts after, and of those that start after it.
fn edges_of(conn: &Connection, work_id: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT c.board_num, b.slug, b.is_default
           FROM (SELECT after_id AS other FROM db_work_deps WHERE work_id = ?1
                 UNION SELECT work_id FROM db_work_deps WHERE after_id = ?1) e
           JOIN db_work_cards c ON c.work_id = e.other
           JOIN db_boards b ON b.id = c.board_id
          ORDER BY c.board_num",
    )?;
    let rows = stmt.query_map(params![work_id], |r| {
        let (num, slug, default): (i64, String, bool) = (r.get(0)?, r.get(1)?, r.get(2)?);
        Ok(if default { format!("#{num}") } else { format!("{slug}#{num}") })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Remove a task and everything kept for it.
fn purge_task(conn: &Connection, work_id: &str) -> Result<(), StoreError> {
    for sql in [
        "DELETE FROM db_work_deps WHERE work_id = ?1 OR after_id = ?1",
        "DELETE FROM db_work_events WHERE work_id = ?1",
        "DELETE FROM db_work_prs WHERE work_id = ?1",
        "DELETE FROM db_work_docs WHERE work_id = ?1",
        "DELETE FROM db_work_media WHERE work_id = ?1",
        "DELETE FROM db_work_ui WHERE work_id = ?1",
        "DELETE FROM db_work_cards WHERE work_id = ?1",
        "DELETE FROM db_work_queue WHERE id = ?1",
    ] {
        conn.execute(sql, params![work_id])?;
    }
    Ok(())
}

/// Whether `slug` can be a board's (`except` is the board asking, which may keep its own).
fn slug_free(conn: &Connection, slug: &str, except: &str, now_ms: i64) -> Result<(), BoardError> {
    if !valid_slug(slug) {
        return refused(format!("a slug is lowercase letters, digits and hyphens, up to {MAX_SLUG}: \"{slug}\""));
    }
    if slug == ALL {
        return refused("\"all\" is not a board name; it means every board");
    }
    match find(conn, slug, now_ms)? {
        Some(other) if other.id != except && (other.slug == slug || other.aliases.iter().any(|a| a.slug == slug && a.until > now_ms)) => {
            refused(format!("the slug \"{slug}\" is already used by board {}", other.name))
        }
        _ => Ok(()),
    }
}

/// SQL: the card `c` is on a board an agent may work on (not archived, and its list is empty or
/// has the agent). `id` and `uid` are the placeholders the caller bound for the agent's id and
/// UID. A card with no board yet (an older build's, numbered on first sight) is on the default
/// board; with no default board at all it counts as visible.
pub(crate) fn card_visible_to_agent(id: &str, uid: &str) -> String {
    format!(
        "(EXISTS (
            SELECT 1 FROM db_boards b
             WHERE b.id = COALESCE(NULLIF(c.board_id, ''), (SELECT id FROM db_boards WHERE is_default = 1 LIMIT 1))
               AND b.archived = 0
               AND (b.agents = '' OR instr(b.agents, '\"' || {id} || '\"') > 0
                    OR ({uid} <> '' AND instr(b.agents, '\"' || {uid} || '\"') > 0)))
         OR (COALESCE(c.board_id, '') = '' AND NOT EXISTS (SELECT 1 FROM db_boards WHERE is_default = 1)))"
    )
}

#[cfg(test)]
#[path = "work_boards_tests.rs"]
mod tests;
