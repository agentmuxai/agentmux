# Jekt & Forge — Full Implementation Spec

**Author:** AgentX
**Date:** 2026-03-08
**Status:** Draft
**Base Version:** 0.31.80
**Depends On:** `specs/jekt-and-agent-management.md` (architecture spec)

---

## Table of Contents

1. [Overview](#1-overview)
2. [Phase 0 — Wire Jekt (Immediate Fix)](#2-phase-0--wire-jekt)
3. [Phase 2 — Agent Config Storage](#3-phase-2--agent-config-storage)
4. [Phase 3 — Forge UI (Basic)](#4-phase-3--forge-ui-basic)
5. [Phase 1 — JektRouter Unification](#5-phase-1--jektrouter-unification)
6. [Phase 4 — Skills + Launch](#6-phase-4--skills--launch)
7. [Phase 5 — LAN Discovery (Tier 2)](#7-phase-5--lan-discovery-tier-2)
8. [Phase 6 — Cloud Relay (Tier 3)](#8-phase-6--cloud-relay-tier-3)
9. [Testing Strategy](#9-testing-strategy)
10. [Migration Notes](#10-migration-notes)

---

## 1. Overview

### Execution Order

```
Phase 0 → Phase 2 → Phase 3 → Phase 1 → Phase 4 → Phase 5 → Phase 6
(fix)    (storage)  (UI)     (router)   (launch)   (LAN)    (cloud)
```

### Architecture Snapshot

```
┌────────────────────────────────────┐
│         Frontend (React 19)        │
│  ┌──────────┐  ┌──────────────┐   │
│  │ Forge UI │  │ Agent/Term   │   │
│  │ (new)    │  │ (existing)   │   │
│  └────┬─────┘  └──────┬──────┘   │
│       │                │          │
│       └───── RPC ──────┘          │
├────────────────────────────────────┤
│       Backend (Rust/Axum)          │
│  ┌──────────┐  ┌──────────────┐   │
│  │ Forge    │  │ JektRouter   │   │
│  │ Storage  │  │ (unified)    │   │
│  └────┬─────┘  └──────┬──────┘   │
│       │                │          │
│  ┌────┴────┐  ┌────────┴───────┐  │
│  │ SQLite  │  │ BlockController│  │
│  │ (wave)  │  │ (PTY write)   │  │
│  └─────────┘  └────────────────┘  │
└────────────────────────────────────┘
```

### Conventions

- **Rust files:** `agentmuxsrv-rs/src/` prefix (abbreviated as `rs/src/`)
- **Frontend files:** `frontend/app/` prefix (abbreviated as `fe/app/`)
- **New files:** Marked with `[NEW]`
- **Modified files:** Marked with `[MOD]`
- **Code blocks:** Show exact code to add/change with surrounding context

---

## 2. Phase 0 — Wire Jekt

**Goal:** Get local jekt working end-to-end. One file, one change.

### 2.1 Files

| File | Action |
|------|--------|
| `rs/src/main.rs` | `[MOD]` Add `set_input_sender()` call + import |

### 2.2 Changes

**`rs/src/main.rs`**

Add import:
```rust
// Add after existing `use backend::...` imports (line ~20)
use backend::blockcontroller;
```

Add wiring after reactive handler creation (after line 137):
```rust
    // Reactive handler (global singleton) + poller
    let reactive_handler = reactive::get_global_handler();

    // Wire reactive handler to block controller for terminal injection (jekt)
    reactive_handler.set_input_sender(Arc::new(|block_id: &str, data: &[u8]| {
        blockcontroller::send_input(
            block_id,
            blockcontroller::BlockInputUnion::data(data.to_vec()),
        )
    }));

    let poller = Arc::new(Poller::new(
```

### 2.3 Verification

```bash
# Build backend
task build:backend

# Start dev mode
task dev

# In another terminal, register an agent and inject a message:
# 1. Open a terminal pane in AgentMux
# 2. Note the block_id from the URL/tab
# 3. Register:
curl -X POST http://localhost:<port>/wave/reactive/register \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "test-agent", "block_id": "<block-id>"}'

# 4. Inject:
curl -X POST http://localhost:<port>/wave/reactive/inject \
  -H "Content-Type: application/json" \
  -d '{"target_agent": "test-agent", "message": "echo hello from jekt"}'

# 5. Verify "echo hello from jekt" + Enter appears in the terminal pane
```

### 2.4 Enter Key Investigation

The current handler sends message and `\r` separately with a 150ms `thread::sleep` between them. This blocks the Mutex. Test both approaches:

**Option A — Single payload (preferred if it works):**
```rust
// In handler.rs inject_message(), replace the two-step send:
let payload = format!("{}\r", final_msg);
sender(&block_id, payload.as_bytes())?;
```

**Option B — Async delay (if PTY needs separation):**
```rust
// Replace std::thread::sleep with tokio channel-based approach
// Send both payloads through a channel, let an async task handle the delay
// This avoids blocking the Mutex
```

Test Option A first. If the Enter key doesn't register in all shells (bash, zsh, pwsh), fall back to Option B.

---

## 3. Phase 2 — Agent Config Storage

**Goal:** Persist agent configurations in SQLite. Backend-only, no UI yet.

### 3.1 Files

| File | Action |
|------|--------|
| `rs/src/backend/forge/mod.rs` | `[NEW]` Module root, re-exports |
| `rs/src/backend/forge/types.rs` | `[NEW]` Data types (AgentConfig, Skill, etc.) |
| `rs/src/backend/forge/storage.rs` | `[NEW]` SQLite CRUD operations |
| `rs/src/backend/mod.rs` | `[MOD]` Add `pub mod forge;` |
| `rs/src/backend/storage/wstore.rs` | `[MOD]` Add migration for forge tables |
| `rs/src/server/mod.rs` | `[MOD]` Add forge HTTP routes |
| `rs/src/server/forge.rs` | `[NEW]` HTTP handlers for forge API |
| `rs/src/main.rs` | `[MOD]` Initialize forge storage |

### 3.2 Data Types

**`rs/src/backend/forge/types.rs`**

```rust
use serde::{Deserialize, Serialize};

/// Agent provider definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProvider {
    pub id: String,               // "claude" | "gemini" | "codex" | "custom"
    pub display_name: String,     // "Claude Code"
    pub cli_command: String,      // "claude"
    pub cli_args: Vec<String>,    // Default args
    pub agentmd_filename: String, // "CLAUDE.md"
    pub auth_check_command: Option<String>,
    pub supports_mcp: bool,
}

/// Core agent configuration (stored in SQLite).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub working_directory: String,
    pub shell: String,
    pub provider_flags: Vec<String>,
    pub env_vars: std::collections::HashMap<String, String>,
    pub auto_start: bool,
    pub restart_on_crash: bool,
    pub idle_timeout_minutes: i64,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_launched_at: Option<i64>,
}

/// Content types stored alongside agent configs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ContentType {
    #[serde(rename = "agentmd")]
    AgentMd,
    #[serde(rename = "soul")]
    Soul,
    #[serde(rename = "skills")]
    Skills,
    #[serde(rename = "mcp")]
    Mcp,
    #[serde(rename = "env")]
    Env,
}

impl ContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentMd => "agentmd",
            Self::Soul => "soul",
            Self::Skills => "skills",
            Self::Mcp => "mcp",
            Self::Env => "env",
        }
    }
}

/// Agent content (large text stored separately).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContent {
    pub agent_id: String,
    pub content_type: String,
    pub content: String,
    pub updated_at: i64,
}

/// Skill definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub trigger: String,
    #[serde(rename = "type")]
    pub skill_type: String,  // "command" | "prompt" | "workflow" | "mcp"
    pub command: Option<String>,
    pub template: Option<String>,
    pub parameters: Option<serde_json::Value>,
}

/// Agent runtime state (not persisted across server restarts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRuntime {
    pub agent_id: String,
    pub block_id: Option<String>,
    pub tab_id: Option<String>,
    pub status: String,  // created, launching, running, stopping, stopped, errored, crashed
    pub started_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub error_message: Option<String>,
}

/// API request/response types.

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub provider_id: String,
    pub working_directory: Option<String>,
    pub shell: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    pub name: Option<String>,
    pub provider_id: Option<String>,
    pub working_directory: Option<String>,
    pub shell: Option<String>,
    pub provider_flags: Option<Vec<String>>,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub auto_start: Option<bool>,
    pub restart_on_crash: Option<bool>,
    pub idle_timeout_minutes: Option<i64>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct SetContentRequest {
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct LaunchAgentRequest {
    pub tab_id: Option<String>,  // Which tab to launch in (new tab if None)
}
```

### 3.3 SQLite Storage

**`rs/src/backend/forge/storage.rs`**

```rust
use rusqlite::{params, Connection};
use super::types::*;

/// Run forge table migrations.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS forge_agents (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            provider_id TEXT NOT NULL,
            working_directory TEXT NOT NULL DEFAULT '',
            shell TEXT NOT NULL DEFAULT 'bash',
            provider_flags TEXT NOT NULL DEFAULT '[]',
            env_vars TEXT NOT NULL DEFAULT '{}',
            auto_start INTEGER NOT NULL DEFAULT 0,
            restart_on_crash INTEGER NOT NULL DEFAULT 0,
            idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '[]',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_launched_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS forge_content (
            agent_id TEXT NOT NULL,
            content_type TEXT NOT NULL,
            content TEXT NOT NULL DEFAULT '',
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (agent_id, content_type),
            FOREIGN KEY (agent_id) REFERENCES forge_agents(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS forge_skills (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            trigger TEXT NOT NULL,
            skill_type TEXT NOT NULL,
            config TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS forge_agent_skills (
            agent_id TEXT NOT NULL,
            skill_id TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, skill_id),
            FOREIGN KEY (agent_id) REFERENCES forge_agents(id) ON DELETE CASCADE,
            FOREIGN KEY (skill_id) REFERENCES forge_skills(id) ON DELETE CASCADE
        );"
    )
}

/// CRUD operations for agent configs.

pub fn create_agent(conn: &Connection, req: &CreateAgentRequest) -> rusqlite::Result<AgentConfig> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let shell = req.shell.as_deref().unwrap_or("bash");
    let working_dir = req.working_directory.as_deref().unwrap_or("");
    let tags = serde_json::to_string(&req.tags.as_deref().unwrap_or(&[])).unwrap();

    conn.execute(
        "INSERT INTO forge_agents (id, name, provider_id, working_directory, shell, tags, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![id, req.name, req.provider_id, working_dir, shell, tags, now],
    )?;

    get_agent(conn, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn get_agent(conn: &Connection, id: &str) -> rusqlite::Result<Option<AgentConfig>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, provider_id, working_directory, shell, provider_flags, env_vars,
                auto_start, restart_on_crash, idle_timeout_minutes, tags,
                created_at, updated_at, last_launched_at
         FROM forge_agents WHERE id = ?1"
    )?;

    let result = stmt.query_row(params![id], |row| {
        Ok(AgentConfig {
            id: row.get(0)?,
            name: row.get(1)?,
            provider_id: row.get(2)?,
            working_directory: row.get(3)?,
            shell: row.get(4)?,
            provider_flags: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
            env_vars: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
            auto_start: row.get::<_, i64>(7)? != 0,
            restart_on_crash: row.get::<_, i64>(8)? != 0,
            idle_timeout_minutes: row.get(9)?,
            tags: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or_default(),
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            last_launched_at: row.get(13)?,
        })
    });

    match result {
        Ok(config) => Ok(Some(config)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn get_agent_by_name(conn: &Connection, name: &str) -> rusqlite::Result<Option<AgentConfig>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM forge_agents WHERE name = ?1"
    )?;
    match stmt.query_row(params![name], |row| row.get::<_, String>(0)) {
        Ok(id) => get_agent(conn, &id),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn list_agents(conn: &Connection) -> rusqlite::Result<Vec<AgentConfig>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM forge_agents ORDER BY name"
    )?;
    let ids: Vec<String> = stmt.query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut agents = Vec::new();
    for id in ids {
        if let Some(agent) = get_agent(conn, &id)? {
            agents.push(agent);
        }
    }
    Ok(agents)
}

pub fn update_agent(conn: &Connection, id: &str, req: &UpdateAgentRequest) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();

    // Build dynamic UPDATE SET clauses
    let mut sets = vec!["updated_at = ?1".to_string()];
    let mut param_idx = 2u32;

    // We'll build the query dynamically based on which fields are present
    // For simplicity, fetch current, merge, and write back
    let current = get_agent(conn, id)?
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;

    let name = req.name.as_deref().unwrap_or(&current.name);
    let provider_id = req.provider_id.as_deref().unwrap_or(&current.provider_id);
    let working_directory = req.working_directory.as_deref().unwrap_or(&current.working_directory);
    let shell = req.shell.as_deref().unwrap_or(&current.shell);
    let provider_flags = serde_json::to_string(
        req.provider_flags.as_ref().unwrap_or(&current.provider_flags)
    ).unwrap();
    let env_vars = serde_json::to_string(
        req.env_vars.as_ref().unwrap_or(&current.env_vars)
    ).unwrap();
    let auto_start = req.auto_start.unwrap_or(current.auto_start) as i64;
    let restart_on_crash = req.restart_on_crash.unwrap_or(current.restart_on_crash) as i64;
    let idle_timeout = req.idle_timeout_minutes.unwrap_or(current.idle_timeout_minutes);
    let tags = serde_json::to_string(
        req.tags.as_ref().unwrap_or(&current.tags)
    ).unwrap();

    conn.execute(
        "UPDATE forge_agents SET
            name = ?1, provider_id = ?2, working_directory = ?3, shell = ?4,
            provider_flags = ?5, env_vars = ?6, auto_start = ?7,
            restart_on_crash = ?8, idle_timeout_minutes = ?9, tags = ?10,
            updated_at = ?11
         WHERE id = ?12",
        params![name, provider_id, working_directory, shell,
                provider_flags, env_vars, auto_start,
                restart_on_crash, idle_timeout, tags, now, id],
    )?;
    Ok(())
}

pub fn delete_agent(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM forge_agents WHERE id = ?1", params![id])?;
    Ok(())
}

// Content CRUD

pub fn get_content(conn: &Connection, agent_id: &str, content_type: &str) -> rusqlite::Result<Option<AgentContent>> {
    let mut stmt = conn.prepare(
        "SELECT agent_id, content_type, content, updated_at
         FROM forge_content WHERE agent_id = ?1 AND content_type = ?2"
    )?;
    match stmt.query_row(params![agent_id, content_type], |row| {
        Ok(AgentContent {
            agent_id: row.get(0)?,
            content_type: row.get(1)?,
            content: row.get(2)?,
            updated_at: row.get(3)?,
        })
    }) {
        Ok(c) => Ok(Some(c)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn set_content(conn: &Connection, agent_id: &str, content_type: &str, content: &str) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO forge_content (agent_id, content_type, content, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(agent_id, content_type) DO UPDATE SET content = ?3, updated_at = ?4",
        params![agent_id, content_type, content, now],
    )?;
    Ok(())
}

pub fn get_all_content(conn: &Connection, agent_id: &str) -> rusqlite::Result<Vec<AgentContent>> {
    let mut stmt = conn.prepare(
        "SELECT agent_id, content_type, content, updated_at
         FROM forge_content WHERE agent_id = ?1"
    )?;
    let content = stmt.query_map(params![agent_id], |row| {
        Ok(AgentContent {
            agent_id: row.get(0)?,
            content_type: row.get(1)?,
            content: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })?.filter_map(|r| r.ok()).collect();
    Ok(content)
}
```

### 3.4 HTTP Handlers

**`rs/src/server/forge.rs`** `[NEW]`

```rust
use axum::{extract::State, extract::Path, extract::Query, Json};
use std::sync::Arc;
use crate::backend::forge::{storage, types::*};
use super::AppState;

pub async fn handle_list_agents(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AgentConfig>>, (axum::http::StatusCode, String)> {
    let conn = state.wstore.conn();
    storage::list_agents(&conn)
        .map(Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AgentConfig>, (axum::http::StatusCode, String)> {
    let conn = state.wstore.conn();
    storage::get_agent(&conn, &id)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(Json)
        .ok_or((axum::http::StatusCode::NOT_FOUND, "agent not found".into()))
}

pub async fn handle_create_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Json<AgentConfig>, (axum::http::StatusCode, String)> {
    let conn = state.wstore.conn();
    storage::create_agent(&conn, &req)
        .map(Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_update_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<(), (axum::http::StatusCode, String)> {
    let conn = state.wstore.conn();
    storage::update_agent(&conn, &id, &req)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_delete_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<(), (axum::http::StatusCode, String)> {
    let conn = state.wstore.conn();
    storage::delete_agent(&conn, &id)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_get_content(
    State(state): State<Arc<AppState>>,
    Path((id, content_type)): Path<(String, String)>,
) -> Result<Json<Option<AgentContent>>, (axum::http::StatusCode, String)> {
    let conn = state.wstore.conn();
    storage::get_content(&conn, &id, &content_type)
        .map(Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_set_content(
    State(state): State<Arc<AppState>>,
    Path((id, content_type)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<(), (axum::http::StatusCode, String)> {
    let content = body.get("content")
        .and_then(|v| v.as_str())
        .ok_or((axum::http::StatusCode::BAD_REQUEST, "missing content field".into()))?;
    let conn = state.wstore.conn();
    storage::set_content(&conn, &id, &content_type, content)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Open a file in the system editor.
pub async fn handle_open_editor(
    Path((id, content_type)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let conn = state.wstore.conn();

    // Get agent config for working directory
    let agent = storage::get_agent(&conn, &id)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((axum::http::StatusCode::NOT_FOUND, "agent not found".into()))?;

    // Determine file path
    let data_dir = crate::backend::wavebase::get_wave_data_dir();
    let agent_dir = data_dir.join("agents").join(&id);
    std::fs::create_dir_all(&agent_dir)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let filename = match content_type.as_str() {
        "agentmd" => agent_provider_filename(&agent.provider_id),
        "soul" => "soul.md",
        "skills" => "skills.json",
        "mcp" => "mcp.json",
        _ => return Err((axum::http::StatusCode::BAD_REQUEST, "invalid content type".into())),
    };

    let file_path = agent_dir.join(filename);

    // Write current content to file if it doesn't exist
    if !file_path.exists() {
        let content = storage::get_content(&conn, &id, &content_type)
            .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let text = content.map(|c| c.content).unwrap_or_default();
        std::fs::write(&file_path, &text)
            .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    // Open in system editor
    open_in_editor(&file_path)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "file_path": file_path.display().to_string(),
        "filename": filename,
    })))
}

fn agent_provider_filename(provider_id: &str) -> &'static str {
    match provider_id {
        "claude" => "CLAUDE.md",
        "gemini" => "GEMINI.md",
        "codex" => "CODEX.md",
        _ => "AGENT.md",
    }
}

fn open_in_editor(path: &std::path::Path) -> Result<(), String> {
    let path_str = path.display().to_string();

    // Check $VISUAL, then $EDITOR, then system default
    if let Ok(editor) = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")) {
        std::process::Command::new(&editor)
            .arg(&path_str)
            .spawn()
            .map_err(|e| format!("failed to open editor '{}': {}", editor, e))?;
        return Ok(());
    }

    // System default
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path_str])
            .spawn()
            .map_err(|e| format!("failed to open file: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path_str)
            .spawn()
            .map_err(|e| format!("failed to open file: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path_str)
            .spawn()
            .map_err(|e| format!("failed to open file: {}", e))?;
    }
    Ok(())
}
```

### 3.5 Route Registration

**`rs/src/server/mod.rs`** `[MOD]`

Add to router builder:
```rust
// Forge API (auth required)
.route("/api/forge/agents", get(forge::handle_list_agents).post(forge::handle_create_agent))
.route("/api/forge/agents/:id", get(forge::handle_get_agent).patch(forge::handle_update_agent).delete(forge::handle_delete_agent))
.route("/api/forge/agents/:id/content/:content_type", get(forge::handle_get_content).put(forge::handle_set_content))
.route("/api/forge/agents/:id/edit/:content_type", post(forge::handle_open_editor))
```

### 3.6 Migration in WaveStore

**`rs/src/backend/storage/wstore.rs`** `[MOD]`

Add to the `WaveStore::open()` migration sequence:
```rust
// After existing table creation
crate::backend::forge::storage::migrate(&conn)?;
```

### 3.7 File Watching for Auto-Refresh

**`rs/src/backend/forge/watcher.rs`** `[NEW]`

Use the existing `notify` crate (already a dependency) to watch agent content files on disk and sync changes back to SQLite:

```rust
use notify::{Watcher, RecursiveMode, Event, EventKind};
use std::path::PathBuf;
use std::sync::Arc;

/// Watch agent content files for external edits.
/// When a file is modified, read its content and update the database.
pub fn spawn_content_watcher(
    wstore: Arc<crate::backend::storage::wstore::WaveStore>,
    data_dir: PathBuf,
    event_bus: Arc<crate::backend::eventbus::EventBus>,
) -> notify::Result<notify::RecommendedWatcher> {
    let agents_dir = data_dir.join("agents");
    std::fs::create_dir_all(&agents_dir)?;

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Modify(_)) {
                for path in &event.paths {
                    handle_file_change(path, &wstore, &event_bus);
                }
            }
        }
    })?;

    watcher.watch(&agents_dir, RecursiveMode::Recursive)?;
    Ok(watcher)
}

fn handle_file_change(
    path: &std::path::Path,
    wstore: &crate::backend::storage::wstore::WaveStore,
    event_bus: &crate::backend::eventbus::EventBus,
) {
    // Parse path: agents/{agent_id}/{filename}
    let components: Vec<_> = path.components().collect();
    if components.len() < 2 { return; }

    let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
    let agent_id = path.parent()
        .and_then(|p| p.file_name())
        .and_then(|f| f.to_str())
        .unwrap_or("");

    let content_type = match filename {
        "CLAUDE.md" | "GEMINI.md" | "CODEX.md" | "AGENT.md" => "agentmd",
        "soul.md" => "soul",
        "skills.json" => "skills",
        "mcp.json" => "mcp",
        _ => return,
    };

    // Read file and update database
    if let Ok(content) = std::fs::read_to_string(path) {
        let conn = wstore.conn();
        let _ = super::storage::set_content(&conn, agent_id, content_type, &content);

        // Broadcast change event to frontend
        event_bus.broadcast(serde_json::json!({
            "type": "forge:content-changed",
            "agent_id": agent_id,
            "content_type": content_type,
        }).to_string());
    }
}
```

---

## 4. Phase 3 — Forge UI (Basic)

**Goal:** Functional Forge pane in AgentMux with agent list, detail panel, and read-only content views.

### 4.1 Files

| File | Action |
|------|--------|
| `fe/app/view/forge/forge-model.ts` | `[NEW]` ViewModel + Jotai atoms |
| `fe/app/view/forge/forge-view.tsx` | `[NEW]` Main Forge component |
| `fe/app/view/forge/agent-list.tsx` | `[NEW]` Agent sidebar list |
| `fe/app/view/forge/agent-detail.tsx` | `[NEW]` Agent detail panel |
| `fe/app/view/forge/content-preview.tsx` | `[NEW]` Read-only markdown preview |
| `fe/app/view/forge/forge.scss` | `[NEW]` Forge styles |
| `fe/app/view/forge/index.ts` | `[NEW]` Module exports |
| `fe/app/view/forge/types.ts` | `[NEW]` TypeScript types |
| `fe/app/view/forge/api.ts` | `[NEW]` Backend API client |

### 4.2 TypeScript Types

**`fe/app/view/forge/types.ts`**

```typescript
export interface AgentConfig {
  id: string;
  name: string;
  provider_id: string;
  working_directory: string;
  shell: string;
  provider_flags: string[];
  env_vars: Record<string, string>;
  auto_start: boolean;
  restart_on_crash: boolean;
  idle_timeout_minutes: number;
  tags: string[];
  created_at: number;
  updated_at: number;
  last_launched_at: number | null;
}

export interface AgentContent {
  agent_id: string;
  content_type: string;
  content: string;
  updated_at: number;
}

export type ContentType = "agentmd" | "soul" | "skills" | "mcp" | "env";

export interface AgentProvider {
  id: string;
  display_name: string;
  cli_command: string;
  agentmd_filename: string;
  icon: string;  // React icon component name
}

export const PROVIDERS: AgentProvider[] = [
  { id: "claude", display_name: "Claude Code", cli_command: "claude", agentmd_filename: "CLAUDE.md", icon: "sparkles" },
  { id: "gemini", display_name: "Gemini CLI", cli_command: "gemini", agentmd_filename: "GEMINI.md", icon: "gem" },
  { id: "codex", display_name: "Codex CLI", cli_command: "codex", agentmd_filename: "CODEX.md", icon: "robot" },
  { id: "custom", display_name: "Custom", cli_command: "", agentmd_filename: "AGENT.md", icon: "terminal" },
];

export type AgentStatus = "created" | "launching" | "running" | "stopping" | "stopped" | "errored" | "crashed";
```

### 4.3 API Client

**`fe/app/view/forge/api.ts`**

```typescript
import { getApi } from "@/app/store/global";

const BASE = "/api/forge";

export async function listAgents(): Promise<AgentConfig[]> {
  const resp = await fetch(`${BASE}/agents`, { headers: authHeaders() });
  return resp.json();
}

export async function getAgent(id: string): Promise<AgentConfig> {
  const resp = await fetch(`${BASE}/agents/${id}`, { headers: authHeaders() });
  return resp.json();
}

export async function createAgent(req: { name: string; provider_id: string; working_directory?: string }): Promise<AgentConfig> {
  const resp = await fetch(`${BASE}/agents`, {
    method: "POST",
    headers: { ...authHeaders(), "Content-Type": "application/json" },
    body: JSON.stringify(req),
  });
  return resp.json();
}

export async function updateAgent(id: string, req: Partial<AgentConfig>): Promise<void> {
  await fetch(`${BASE}/agents/${id}`, {
    method: "PATCH",
    headers: { ...authHeaders(), "Content-Type": "application/json" },
    body: JSON.stringify(req),
  });
}

export async function deleteAgent(id: string): Promise<void> {
  await fetch(`${BASE}/agents/${id}`, { method: "DELETE", headers: authHeaders() });
}

export async function getContent(agentId: string, contentType: string): Promise<AgentContent | null> {
  const resp = await fetch(`${BASE}/agents/${agentId}/content/${contentType}`, { headers: authHeaders() });
  return resp.json();
}

export async function setContent(agentId: string, contentType: string, content: string): Promise<void> {
  await fetch(`${BASE}/agents/${agentId}/content/${contentType}`, {
    method: "PUT",
    headers: { ...authHeaders(), "Content-Type": "application/json" },
    body: JSON.stringify({ content }),
  });
}

export async function openInEditor(agentId: string, contentType: string): Promise<{ file_path: string }> {
  const resp = await fetch(`${BASE}/agents/${agentId}/edit/${contentType}`, {
    method: "POST",
    headers: authHeaders(),
  });
  return resp.json();
}

function authHeaders(): Record<string, string> {
  // Use the same auth mechanism as existing API calls
  const authKey = getApi()?.getAuthKey?.() ?? "";
  return { "X-AuthKey": authKey };
}
```

### 4.4 Main Forge View

**`fe/app/view/forge/forge-view.tsx`**

```tsx
import React, { useState, useEffect } from "react";
import { AgentList } from "./agent-list";
import { AgentDetail } from "./agent-detail";
import { listAgents, createAgent, deleteAgent } from "./api";
import type { AgentConfig } from "./types";
import "./forge.scss";

export function ForgeView() {
  const [agents, setAgents] = useState<AgentConfig[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    const list = await listAgents();
    setAgents(list);
    setLoading(false);
  };

  useEffect(() => { refresh(); }, []);

  // Listen for content-changed events from backend file watcher
  useEffect(() => {
    // Subscribe to EventBus "forge:content-changed" events
    // Trigger re-fetch of content when files are edited externally
    // Implementation depends on existing EventBus subscription pattern
  }, []);

  const handleCreate = async (name: string, providerId: string) => {
    const agent = await createAgent({ name, provider_id: providerId });
    await refresh();
    setSelectedId(agent.id);
  };

  const handleDelete = async (id: string) => {
    await deleteAgent(id);
    if (selectedId === id) setSelectedId(null);
    await refresh();
  };

  const selected = agents.find(a => a.id === selectedId) ?? null;

  return (
    <div className="forge-view">
      <div className="forge-header">
        <h2>The Forge</h2>
      </div>
      <div className="forge-body">
        <AgentList
          agents={agents}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onCreate={handleCreate}
          loading={loading}
        />
        <AgentDetail
          agent={selected}
          onDelete={handleDelete}
          onUpdate={refresh}
        />
      </div>
    </div>
  );
}
```

### 4.5 Content Preview (Read-Only + Edit Button)

**`fe/app/view/forge/content-preview.tsx`**

```tsx
import React, { useState, useEffect } from "react";
import ReactMarkdown from "react-markdown";
import { getContent, openInEditor } from "./api";
import type { ContentType } from "./types";

interface Props {
  agentId: string;
  contentType: ContentType;
  label: string;
}

export function ContentPreview({ agentId, contentType, label }: Props) {
  const [content, setContent] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [filePath, setFilePath] = useState<string | null>(null);

  useEffect(() => {
    setLoading(true);
    getContent(agentId, contentType).then(c => {
      setContent(c?.content ?? "");
      setLoading(false);
    });
  }, [agentId, contentType]);

  const handleEdit = async () => {
    const result = await openInEditor(agentId, contentType);
    setFilePath(result.file_path);
  };

  if (loading) return <div className="content-preview loading">Loading...</div>;

  return (
    <div className="content-preview">
      <div className="content-preview-header">
        <span className="content-label">{label}</span>
        <button className="edit-button" onClick={handleEdit} title="Open in editor">
          ✎ Edit
        </button>
      </div>
      <div className="content-preview-body">
        {content ? (
          contentType === "skills" || contentType === "mcp" ? (
            <pre className="content-json">{content}</pre>
          ) : (
            <ReactMarkdown>{content}</ReactMarkdown>
          )
        ) : (
          <div className="content-empty">
            No {label.toLowerCase()} configured.
            <button onClick={handleEdit}>Create</button>
          </div>
        )}
      </div>
      {filePath && (
        <div className="content-filepath">
          Editing: <code>{filePath}</code>
        </div>
      )}
    </div>
  );
}
```

### 4.6 Block View Registration

Register the Forge as a new block view type so it can be opened as a pane.

**Existing pattern** (from `agent-view.tsx`): Views are registered by the `view` key in block metadata. The view routing system needs to recognize `"forge"` and render `ForgeView`.

The exact registration point depends on how the view router is implemented. Look for where `"agent"`, `"term"`, `"chat"`, etc. are mapped to React components. Add:

```typescript
case "forge":
  return <ForgeView />;
```

---

## 5. Phase 1 — JektRouter Unification

**Goal:** Replace ReactiveHandler + MessageBus dual inject paths with a single JektRouter.

### 5.1 Files

| File | Action |
|------|--------|
| `rs/src/backend/jekt/mod.rs` | `[NEW]` Module root |
| `rs/src/backend/jekt/router.rs` | `[NEW]` JektRouter struct |
| `rs/src/backend/jekt/types.rs` | `[NEW]` Jekt types |
| `rs/src/backend/mod.rs` | `[MOD]` Add `pub mod jekt;` |
| `rs/src/main.rs` | `[MOD]` Create JektRouter, add to AppState |
| `rs/src/server/mod.rs` | `[MOD]` Add `/api/jekt/*` routes, update AppState |
| `rs/src/server/jekt.rs` | `[NEW]` HTTP handlers |
| `rs/src/server/websocket.rs` | `[MOD]` `bus:inject` delegates to JektRouter |

### 5.2 JektRouter

**`rs/src/backend/jekt/router.rs`**

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use super::types::*;
use crate::backend::blockcontroller;
use crate::backend::reactive::sanitize::{sanitize_message, format_injected_message, validate_agent_id};

pub struct JektRouter {
    inner: Mutex<JektRouterInner>,
}

struct JektRouterInner {
    local_agents: HashMap<String, AgentRegistration>,
    agent_to_block: HashMap<String, String>,
    block_to_agent: HashMap<String, String>,
    audit_log: Vec<AuditEntry>,
}

impl JektRouter {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(JektRouterInner {
                local_agents: HashMap::new(),
                agent_to_block: HashMap::new(),
                block_to_agent: HashMap::new(),
                audit_log: Vec::new(),
            }),
        }
    }

    pub fn register(&self, agent_id: &str, block_id: &str, tab_id: Option<&str>) -> Result<(), String> {
        if !validate_agent_id(agent_id) {
            return Err(format!("invalid agent ID: {}", agent_id));
        }
        let mut inner = self.inner.lock().unwrap();

        // Remove old mappings
        if let Some(old_block) = inner.agent_to_block.remove(agent_id) {
            inner.block_to_agent.remove(&old_block);
        }
        if let Some(old_agent) = inner.block_to_agent.remove(block_id) {
            inner.agent_to_block.remove(&old_agent);
            inner.local_agents.remove(&old_agent);
        }

        let now = chrono::Utc::now().timestamp_millis() as u64;
        inner.agent_to_block.insert(agent_id.to_string(), block_id.to_string());
        inner.block_to_agent.insert(block_id.to_string(), agent_id.to_string());
        inner.local_agents.insert(agent_id.to_string(), AgentRegistration {
            agent_id: agent_id.to_string(),
            block_id: block_id.to_string(),
            tab_id: tab_id.map(|s| s.to_string()),
            registered_at: now,
            last_seen: now,
            tier: JektTier::Local,
        });
        Ok(())
    }

    pub fn unregister(&self, agent_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(block_id) = inner.agent_to_block.remove(agent_id) {
            inner.block_to_agent.remove(&block_id);
        }
        inner.local_agents.remove(agent_id);
    }

    pub fn jekt(&self, req: JektRequest) -> JektResponse {
        let sanitized = sanitize_message(&req.message);
        let formatted = format_injected_message(&sanitized, req.source_agent.as_deref(), false);

        let inner = self.inner.lock().unwrap();

        // Tier 1: Local
        if let Some(block_id) = inner.agent_to_block.get(&req.target_agent) {
            let payload = format!("{}\r", formatted);
            match blockcontroller::send_input(
                block_id,
                blockcontroller::BlockInputUnion::data(payload.into_bytes()),
            ) {
                Ok(()) => JektResponse {
                    success: true,
                    tier: JektTier::Local,
                    block_id: Some(block_id.clone()),
                    error: None,
                },
                Err(e) => JektResponse {
                    success: false,
                    tier: JektTier::Local,
                    block_id: Some(block_id.clone()),
                    error: Some(e),
                },
            }
        } else {
            // TODO: Tier 2 (LAN) and Tier 3 (Cloud) in later phases
            JektResponse {
                success: false,
                tier: JektTier::Local,
                block_id: None,
                error: Some(format!("agent '{}' not found", req.target_agent)),
            }
        }
    }

    pub fn list_agents(&self) -> Vec<AgentRegistration> {
        self.inner.lock().unwrap().local_agents.values().cloned().collect()
    }
}
```

### 5.3 Jekt Types

**`rs/src/backend/jekt/types.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JektTier {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "lan")]
    Lan,
    #[serde(rename = "cloud")]
    Cloud,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JektRequest {
    pub target_agent: String,
    pub message: String,
    pub source_agent: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JektResponse {
    pub success: bool,
    pub tier: JektTier,
    pub block_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub agent_id: String,
    pub block_id: String,
    pub tab_id: Option<String>,
    pub registered_at: u64,
    pub last_seen: u64,
    pub tier: JektTier,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub timestamp: u64,
    pub source_agent: Option<String>,
    pub target_agent: String,
    pub tier: JektTier,
    pub success: bool,
    pub error: Option<String>,
}
```

---

## 6. Phase 4 — Skills + Launch

**Goal:** Launch fully-configured agents from the Forge into terminal panes.

### 6.1 Launch Flow (Backend)

**`rs/src/backend/forge/launch.rs`** `[NEW]`

```rust
use std::collections::HashMap;
use crate::backend::forge::storage;
use crate::backend::storage::wstore::WaveStore;
use crate::backend::waveobj::{Block, MetaMapType};
use crate::backend::wcore;
use crate::backend::blockcontroller;

pub struct LaunchResult {
    pub block_id: String,
    pub tab_id: String,
}

/// Launch an agent: write config files to disk, create block, start controller.
pub fn launch_agent(
    wstore: &WaveStore,
    agent_id: &str,
    tab_id: Option<&str>,
) -> Result<LaunchResult, String> {
    let conn = wstore.conn();

    // 1. Load agent config
    let agent = storage::get_agent(&conn, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or("agent not found")?;

    // 2. Load all content
    let agentmd = storage::get_content(&conn, agent_id, "agentmd")
        .map_err(|e| e.to_string())?;
    let soul = storage::get_content(&conn, agent_id, "soul")
        .map_err(|e| e.to_string())?;
    let mcp = storage::get_content(&conn, agent_id, "mcp")
        .map_err(|e| e.to_string())?;

    // 3. Prepare working directory
    let work_dir = if agent.working_directory.is_empty() {
        dirs::home_dir().unwrap_or_default().display().to_string()
    } else {
        agent.working_directory.clone()
    };
    std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;

    // 4. Write AgentMD file (Soul prepended to AgentMD)
    let agentmd_filename = match agent.provider_id.as_str() {
        "claude" => "CLAUDE.md",
        "gemini" => "GEMINI.md",
        "codex" => "CODEX.md",
        _ => "AGENT.md",
    };

    let mut combined_content = String::new();
    if let Some(soul_content) = &soul {
        if !soul_content.content.is_empty() {
            combined_content.push_str(&soul_content.content);
            combined_content.push_str("\n\n---\n\n");
        }
    }
    if let Some(agentmd_content) = &agentmd {
        combined_content.push_str(&agentmd_content.content);
    }

    let agentmd_path = std::path::Path::new(&work_dir).join(agentmd_filename);
    std::fs::write(&agentmd_path, &combined_content).map_err(|e| e.to_string())?;

    // 5. Write MCP config
    if let Some(mcp_content) = &mcp {
        if !mcp_content.content.is_empty() {
            let mcp_path = std::path::Path::new(&work_dir).join(".mcp.json");
            std::fs::write(&mcp_path, &mcp_content.content).map_err(|e| e.to_string())?;
        }
    }

    // 6. Build block metadata
    let provider = get_provider(&agent.provider_id);
    let mut meta = MetaMapType::new();
    meta.insert("view".into(), serde_json::Value::String("term".into()));
    meta.insert("controller".into(), serde_json::Value::String("cmd".into()));
    meta.insert("cmd".into(), serde_json::Value::String(provider.cli_command.clone()));
    meta.insert("cmd:cwd".into(), serde_json::Value::String(work_dir));
    meta.insert("cmd:runonstart".into(), serde_json::Value::Bool(true));

    if !agent.provider_flags.is_empty() {
        meta.insert("cmd:args".into(), serde_json::to_value(&agent.provider_flags).unwrap());
    }

    // Build env vars with AGENTMUX_AGENT_ID
    let mut env = agent.env_vars.clone();
    env.insert("AGENTMUX_AGENT_ID".into(), agent.name.clone());
    meta.insert("cmd:env".into(), serde_json::to_value(&env).unwrap());

    // Forge metadata
    meta.insert("agent:config_id".into(), serde_json::Value::String(agent.id.clone()));
    meta.insert("agent:provider".into(), serde_json::Value::String(agent.provider_id.clone()));

    // 7. Create block in tab (reuse existing tab or create new)
    // This delegates to existing wcore::create_block() / RPC CreateBlock
    // The exact implementation depends on how blocks are created in the current codebase

    // 8. Update last_launched_at
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE forge_agents SET last_launched_at = ?1 WHERE id = ?2",
        rusqlite::params![now, agent_id],
    ).map_err(|e| e.to_string())?;

    // Return placeholder — actual block creation needs wcore integration
    Ok(LaunchResult {
        block_id: "pending".into(),
        tab_id: tab_id.unwrap_or("pending").into(),
    })
}

struct ProviderDef {
    cli_command: String,
}

fn get_provider(id: &str) -> ProviderDef {
    match id {
        "claude" => ProviderDef { cli_command: "claude".into() },
        "gemini" => ProviderDef { cli_command: "gemini".into() },
        "codex" => ProviderDef { cli_command: "codex".into() },
        _ => ProviderDef { cli_command: "bash".into() },
    }
}
```

### 6.2 Skills Editor (Frontend)

Skills are stored as JSON. The Forge displays them as a formatted list. "Edit" opens `skills.json` in the external editor.

**`fe/app/view/forge/skills-panel.tsx`**

```tsx
import React, { useState, useEffect } from "react";
import { getContent, openInEditor } from "./api";

interface Skill {
  id: string;
  name: string;
  trigger: string;
  type: string;
  description: string;
}

export function SkillsPanel({ agentId }: { agentId: string }) {
  const [skills, setSkills] = useState<Skill[]>([]);

  useEffect(() => {
    getContent(agentId, "skills").then(c => {
      if (c?.content) {
        try { setSkills(JSON.parse(c.content)); } catch {}
      }
    });
  }, [agentId]);

  return (
    <div className="skills-panel">
      <div className="skills-header">
        <span>Skills ({skills.length})</span>
        <button onClick={() => openInEditor(agentId, "skills")}>✎ Edit</button>
      </div>
      <div className="skills-list">
        {skills.length === 0 && <div className="empty">No skills configured.</div>}
        {skills.map(s => (
          <div key={s.id} className="skill-card">
            <div className="skill-name">{s.name}</div>
            <div className="skill-trigger"><code>{s.trigger}</code></div>
            <div className="skill-type">{s.type}</div>
            <div className="skill-desc">{s.description}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
```

---

## 7. Phase 5 — LAN Discovery (Tier 2)

**Goal:** Discover and communicate with AgentMux instances on the same local network.

### 7.1 Files

| File | Action |
|------|--------|
| `rs/src/backend/jekt/lan.rs` | `[NEW]` LAN peer discovery + mDNS |
| `rs/src/backend/jekt/router.rs` | `[MOD]` Add Tier 2 routing |
| `rs/src/server/jekt.rs` | `[MOD]` Add LAN endpoints |
| `rs/Cargo.toml` | `[MOD]` Add `mdns-sd` dependency |

### 7.2 mDNS Service

```rust
// rs/src/backend/jekt/lan.rs
use mdns_sd::{ServiceDaemon, ServiceInfo, ServiceEvent};

const SERVICE_TYPE: &str = "_agentmux._tcp.local.";

pub struct LanDiscovery {
    daemon: ServiceDaemon,
    peers: Arc<Mutex<HashMap<String, LanPeer>>>,
}

pub struct LanPeer {
    pub instance_id: String,
    pub hostname: String,
    pub address: std::net::IpAddr,
    pub port: u16,
    pub version: String,
    pub trusted: bool,
    pub last_seen: u64,
}

impl LanDiscovery {
    pub fn new(instance_id: &str, port: u16, version: &str) -> Result<Self, String> {
        let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;

        // Advertise this instance
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            instance_id,
            &format!("{}.local.", hostname::get().unwrap().to_str().unwrap()),
            "",
            port,
            &[
                ("version", version),
                ("instance_id", instance_id),
            ][..],
        ).map_err(|e| e.to_string())?;

        daemon.register(service).map_err(|e| e.to_string())?;

        // Browse for peers
        let browse = daemon.browse(SERVICE_TYPE).map_err(|e| e.to_string())?;

        let peers = Arc::new(Mutex::new(HashMap::new()));
        let peers_clone = peers.clone();

        // Background listener
        std::thread::spawn(move || {
            while let Ok(event) = browse.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        // Add peer
                        let mut peers = peers_clone.lock().unwrap();
                        if let Some(addr) = info.get_addresses().iter().next() {
                            peers.insert(info.get_fullname().to_string(), LanPeer {
                                instance_id: info.get_property_val_str("instance_id")
                                    .unwrap_or("").to_string(),
                                hostname: info.get_hostname().to_string(),
                                address: *addr,
                                port: info.get_port(),
                                version: info.get_property_val_str("version")
                                    .unwrap_or("").to_string(),
                                trusted: false,
                                last_seen: chrono::Utc::now().timestamp_millis() as u64,
                            });
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, name) => {
                        let mut peers = peers_clone.lock().unwrap();
                        peers.remove(&name);
                    }
                    _ => {}
                }
            }
        });

        Ok(Self { daemon, peers })
    }

    pub fn list_peers(&self) -> Vec<LanPeer> {
        self.peers.lock().unwrap().values().cloned().collect()
    }

    /// Jekt to a peer via HTTP
    pub async fn jekt_to_peer(&self, peer: &LanPeer, req: &JektRequest) -> Result<JektResponse, String> {
        let url = format!("http://{}:{}/api/jekt/inject", peer.address, peer.port);
        let client = reqwest::Client::new();
        let resp = client.post(&url)
            .json(req)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }
}
```

### 7.3 Cargo.toml Addition

```toml
[dependencies]
mdns-sd = "0.11"       # mDNS service discovery
reqwest = { version = "0.12", features = ["json"] }  # HTTP client for LAN peers
hostname = "0.4"        # Get local hostname
```

---

## 8. Phase 6 — Cloud Relay (Tier 3)

**Goal:** WebSocket-based cloud relay for cross-internet jekt.

### 8.1 Files

| File | Action |
|------|--------|
| `rs/src/backend/jekt/cloud.rs` | `[NEW]` Cloud WebSocket relay client |
| `rs/src/backend/jekt/router.rs` | `[MOD]` Add Tier 3 routing |

### 8.2 Cloud Client

```rust
// rs/src/backend/jekt/cloud.rs
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub struct CloudRelay {
    url: String,
    token: String,
    instance_id: String,
    ws_tx: Option<tokio::sync::mpsc::Sender<String>>,
}

impl CloudRelay {
    pub async fn connect(url: &str, token: &str, instance_id: &str) -> Result<Self, String> {
        let ws_url = format!("{}/ws/relay?token={}&instance={}", url, token, instance_id);
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| e.to_string())?;

        let (write, read) = ws_stream.split();
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Spawn writer
        tokio::spawn(async move {
            // Forward channel messages to WebSocket
        });

        // Spawn reader
        tokio::spawn(async move {
            // Read incoming jekt requests from cloud
            // Route them through local JektRouter
        });

        Ok(Self {
            url: url.to_string(),
            token: token.to_string(),
            instance_id: instance_id.to_string(),
            ws_tx: Some(tx),
        })
    }

    pub async fn jekt(&self, target_agent: &str, message: &str) -> Result<(), String> {
        if let Some(tx) = &self.ws_tx {
            let payload = serde_json::json!({
                "type": "jekt",
                "target_agent": target_agent,
                "message": message,
            });
            tx.send(payload.to_string()).await.map_err(|e| e.to_string())?;
            Ok(())
        } else {
            Err("cloud relay not connected".into())
        }
    }
}
```

Cloud relay is the most speculative phase. The exact protocol depends on the cloud service architecture, which needs its own spec.

---

## 9. Testing Strategy

### Phase 0

- **Manual test:** Register agent via curl, inject message, verify it appears in terminal
- **Unit test:** Add test to `reactive/handler.rs` that verifies `inject_message` succeeds when `input_sender` is set

### Phase 2

- **Unit tests:** CRUD operations on `forge_agents`, `forge_content` tables
- **Integration test:** Create agent → set content → get content → delete agent
- **Migration test:** Open existing database, verify forge tables are added without breaking existing data

### Phase 3

- **Component tests:** ForgeView renders agent list, AgentDetail shows correct tabs
- **API tests:** Frontend API client correctly calls backend endpoints
- **E2E test:** Create agent via UI → edit AgentMD → verify file is written → edit externally → verify UI refreshes

### Phase 1

- **Unit tests:** JektRouter register/unregister/jekt
- **Integration test:** Register two agents → jekt from one to other → verify PTY receives input
- **Backward compat test:** Existing `/wave/reactive/inject` still works

### Phase 4

- **E2E test:** Create agent in Forge → Launch → verify terminal starts with correct env vars and CLI
- **Skills test:** Create skills.json → open in editor → verify display updates

### Phase 5-6

- **Network tests:** Two agentmux instances on localhost with different ports → LAN discovery → cross-instance jekt
- **Cloud tests:** Mock relay server → connect → jekt → verify delivery

---

## 10. Migration Notes

### From Claw-Managed Agents

Users with existing Claw-managed agents can import configs:

1. Read existing `CLAUDE.md` from workspace directory
2. Read existing `.mcp.json` from workspace directory
3. Create agent in Forge with matching provider and content
4. Point working_directory to existing workspace

A `forge:import` RPC command will handle this:
```
POST /api/forge/import
{
  "source_directory": "/path/to/agent/workspace",
  "provider_id": "claude",
  "name": "Agent1"
}
```

### Database Compatibility

Forge tables are additive — they don't modify existing `db_*` tables. Safe to deploy incrementally. The `forge_*` tables are created on first startup via migration.

### Backward Compatibility

All existing endpoints continue to work:
- `/wave/reactive/*` — delegates to JektRouter (Phase 1) or remains standalone (Phase 0)
- `/api/bus/*` — MessageBus continues independently
- WebSocket `bus:*` — unchanged

The JektRouter is additive. It doesn't break existing code paths.
