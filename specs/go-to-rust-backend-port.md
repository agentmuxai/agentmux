# Go-to-Rust Backend Port — Full Specification

> **Status:** IN PROGRESS
> **Date:** 2026-02-09
> **Author:** Agent3
> **Branch Pattern:** `agent3/go-to-rust-*`
> **Target:** Eliminate Go sidecar, run backend natively in Tauri's Rust process

---

## Executive Summary

The AgentMux Go backend (`agentmuxsrv`, ~48K LOC across 39 packages) currently runs as a sidecar process. This spec defines the phased port of that backend into Rust, directly inside `src-tauri/src/backend/`. The goal is a **single binary** — no sidecar, no Go runtime, no IPC bridge overhead.

**Why:** Single binary distribution, ~5x less memory, no GC pauses, shared types with Tauri, one language stack.

**Strategy:** Port in layers bottom-up (types → storage → services → controllers → servers), keeping the Go sidecar running until each layer is verified. The frontend switches to Rust endpoints one service at a time.

---

## Completed Work (PR #191, merged)

### Phase 1: Core Types + Storage Layer — DONE

**3,035 lines of Rust, 67 passing tests, 12 files**

| File | Lines | What |
|------|-------|------|
| `backend/mod.rs` | 10 | Module root, re-exports |
| `backend/oref.rs` | 172 | ORef `"type:uuid"` format, custom serde, validation |
| `backend/waveobj.rs` | 687 | WaveObj trait, 7 object types (Client, Window, Workspace, Tab, LayoutState, Block, Temp), merge_meta, helpers |
| `backend/rpc_types.rs` | 303 | RpcMessage wire format, GetMeta/SetMeta/Message command types |
| `backend/storage/mod.rs` | 14 | Storage module exports |
| `backend/storage/error.rs` | 28 | StoreError enum (thiserror) |
| `backend/storage/wstore.rs` | 408 | Generic CRUD: get/insert/update/delete/get_all/count, optimistic locking, VALID_OTYPES SQL injection guard |
| `backend/storage/filestore.rs` | 864 | 64KB part-based file storage, write-through cache, background flusher (tokio), circular files |
| `backend/storage/migrations.rs` | 103 | Schema setup (7 obj tables + 2 file tables) |

**Public API surface:**
- 1 trait (`WaveObj`), 28 structs, 2 enums, 30+ pub functions
- `WaveStore`: `open()`, `get::<T>()`, `must_get::<T>()`, `insert()`, `update()`, `delete()`, `delete_by_otype()`, `get_all::<T>()`, `count::<T>()`
- `FileStore`: `open()`, `make_file()`, `write_file()`, `read_file()`, `append_data()`, `stat()`, `list_files()`, `delete_file()`, `delete_zone()`, `flush_cache()`, `start_flusher()`
- Wire-compatible JSON with Go backend (verified by hardcoded Go JSON → Rust deserialization tests)

---

## Remaining Work — Phase Overview

```
Phase 2: Core Services          ← wcore, wps, eventbus, wconfig
Phase 3: RPC/Communication      ← wshrpc, wshutil, web (WebSocket + HTTP)
Phase 4: Block Controller        ← blockcontroller, shellexec, blocklogger
Phase 5: AI Integration          ← waveai, aiusechat
Phase 6: Remote Connections      ← remote, genconn, wsl, wslconn
Phase 7: Utilities               ← ijson, vdom, suggestion, faviconcache, etc.
Phase 8: Server Entrypoint       ← cmd/server replacement, full Go sidecar elimination
```

---

## Phase 2: Core Services (~1,500 lines)

**Ports:** `wcore` (1,236 LOC), `wps` (338 LOC), `eventbus` (87 LOC), `wconfig` (1,102 LOC)

### 2.1 Event Bus (`backend/eventbus.rs`, ~80 lines)

Port `pkg/eventbus/eventbus.go` — simple pub/sub message bus.

```
Go: global eventbus with Subscribe/Publish
Rust: tokio::sync::broadcast channel
```

- `pub fn subscribe() -> broadcast::Receiver<Event>`
- `pub fn publish(event: Event)`
- Use `tokio::sync::broadcast` (bounded) for the channel

### 2.2 Wave Pub/Sub (`backend/wps.rs`, ~300 lines)

Port `pkg/wps/` — typed pub/sub layer on top of eventbus for WaveObj updates.

- `WaveObjUpdate` already defined in `waveobj.rs`
- `pub fn publish_update(update: WaveObjUpdate)`
- `pub fn subscribe_updates() -> Receiver<WaveObjUpdate>`
- Subscriber filtering by otype/oid

### 2.3 Configuration (`backend/wconfig/`, ~800 lines)

Port `pkg/wconfig/` — settings management with file watching.

| File | Lines | Purpose |
|------|-------|---------|
| `wconfig/mod.rs` | ~50 | Module root |
| `wconfig/settings.rs` | ~400 | Settings types, defaults, get/set |
| `wconfig/watcher.rs` | ~350 | File watcher using `notify` crate |

**New dependency:** `notify = "7"` (file system watcher)

- Settings struct matching Go's SettingsType (JSON)
- `pub fn get_settings() -> Settings`
- `pub fn update_settings(patch: serde_json::Value)`
- `pub fn watch_config(path: PathBuf) -> JoinHandle<()>`
- Debounced file watching (500ms)

### 2.4 Core Coordinator (`backend/wcore.rs`, ~500 lines)

Port `pkg/wcore/` — orchestrates storage + pub/sub + RPC registration.

- `pub fn ensure_initial_data(store: &WaveStore) -> Result<bool, StoreError>` — creates default Client/Window/Workspace/Tab if empty
- `pub fn create_tab(store: &WaveStore, ...) -> Result<Tab, StoreError>`
- `pub fn close_tab(store: &WaveStore, ...) -> Result<(), StoreError>`
- `pub fn create_workspace(store: &WaveStore, ...) -> Result<Workspace, StoreError>`
- `pub fn switch_workspace(store: &WaveStore, ...) -> Result<(), StoreError>`
- Each mutation publishes via wps

**Deliverable:** App can initialize, create/delete tabs and workspaces, publish updates.

---

## Phase 3: RPC/Communication (~2,000 lines)

**Ports:** `wshrpc` (993 LOC), `wshutil` (2,754 LOC), `web` (955 LOC)

This is the **critical integration point** — it's what the frontend talks to.

### 3.1 RPC Type Registry (`backend/rpc/types.rs`, ~400 lines)

Expand `rpc_types.rs` with all command constants and data types from `pkg/wshrpc/wshrpctypes.go`:

- ~60 command constants (COMMAND_*)
- ~40 command data structs (CommandResolveData, CommandCreateBlockData, etc.)
- `RpcResponseData` enum for typed responses

### 3.2 RPC Router (`backend/rpc/router.rs`, ~500 lines)

Port `pkg/wshutil/wshrouter.go`:

- `RpcRouter` struct with command → handler mapping
- `pub fn register<F>(command: &str, handler: F)`
- `pub fn dispatch(msg: RpcMessage) -> Result<RpcMessage, String>`
- Route-based forwarding (route format: `"controller:blockid"`)

### 3.3 WebSocket Server (`backend/server/ws.rs`, ~600 lines)

Port `pkg/web/ws.go` using **tokio-tungstenite** (not tauri-plugin-websocket which is client-only):

**New dependencies:**
- `tokio-tungstenite = "0.26"` (async WebSocket server)
- `futures-util = "0.3"` (stream utilities)

```rust
pub async fn run_ws_server(addr: SocketAddr, router: Arc<RpcRouter>) {
    let listener = TcpListener::bind(addr).await.unwrap();
    while let Ok((stream, _)) = listener.accept().await {
        let router = router.clone();
        tokio::spawn(handle_ws_connection(stream, router));
    }
}
```

- Auth key validation on upgrade
- Per-connection message loop
- Bidirectional RPC (request/response/streaming)
- Connection tracking for broadcast events

### 3.4 HTTP Server (`backend/server/http.rs`, ~400 lines)

Port `pkg/web/web.go` using **axum**:

**New dependency:** `axum = "0.8"` (HTTP framework, already uses tokio)

Routes to implement:
- `GET /wave/file/{zoneid}/{name}` — file streaming
- `POST /wave/file/{zoneid}/{name}` — file upload
- `GET /wave/stream/{blockid}` — SSE streaming for block output
- Auth middleware (X-AuthKey header check)

### 3.5 Tauri Command Bridge (`backend/server/commands.rs`, ~200 lines)

Bridge between Tauri `#[tauri::command]` handlers and the RPC router:

```rust
#[tauri::command]
async fn rpc_call(state: State<'_, AppState>, msg: RpcMessage) -> Result<RpcMessage, String> {
    state.router.dispatch(msg).await.map_err(|e| e.to_string())
}
```

This allows the frontend to call backend services directly via `invoke("rpc_call", { msg })` without going through WebSocket, reducing latency for Tauri-native communication.

**Deliverable:** Frontend can connect via WebSocket or Tauri invoke. Full RPC dispatch works.

---

## Phase 4: Block Controller (~2,000 lines)

**Ports:** `blockcontroller` (1,806 LOC), `shellexec` (871 LOC), `blocklogger` (92 LOC)

### 4.1 Shell Execution (`backend/shellexec/`, ~600 lines)

Port `pkg/shellexec/`:

**New dependency:** `portable-pty = "0.8"` (cross-platform PTY)

- `pub fn spawn_shell(cmd: &str, env: HashMap<String, String>, size: TermSize) -> Result<ShellProc, Error>`
- `ShellProc` wraps PTY master with async read/write
- Handle resize events
- Collect exit status

### 4.2 Block Controller (`backend/blockcontroller/`, ~1,200 lines)

Port `pkg/blockcontroller/`:

- `BlockController` manages lifecycle of each block (terminal, file preview, AI chat)
- `pub fn start_block(block_id: &str, block: &Block) -> Result<(), Error>`
- `pub fn stop_block(block_id: &str) -> Result<(), Error>`
- `pub fn send_input(block_id: &str, data: &[u8]) -> Result<(), Error>`
- `pub fn resize_block(block_id: &str, size: TermSize) -> Result<(), Error>`
- Block output → FileStore (terminal scrollback) + WPS broadcast
- Block type dispatch: `term` → shell, `web` → no controller, `ai` → AI handler

### 4.3 Block Logger (`backend/blocklogger.rs`, ~80 lines)

Port `pkg/blocklogger/` — simple event logger for block operations.

**Deliverable:** Terminal blocks work end-to-end: spawn PTY, pipe I/O, resize, persist output.

---

## Phase 5: AI Integration (~1,500 lines)

**Ports:** `waveai` (1,049 LOC), `aiusechat` (2,478 LOC)

### 5.1 AI Backend Abstraction (`backend/ai/mod.rs`, ~200 lines)

Port `pkg/waveai/`:

- `AIBackend` trait with `send_message()` → streaming response
- Implementations: Anthropic, OpenAI, Google (Gemini)
- Config-driven backend selection

### 5.2 AI Chat Service (`backend/ai/usechat.rs`, ~1,000 lines)

Port `pkg/aiusechat/`:

**New dependencies:**
- `reqwest = { version = "0.12", features = ["json", "stream"] }` (HTTP client)
- `async-stream = "0.3"` (async streaming)

- Tool-use integration (file reading, command execution)
- Streaming response handling
- Message history management
- Command approval system

### 5.3 AI Tool Execution (`backend/ai/tools.rs`, ~300 lines)

- File read tool
- Directory listing tool
- Command execution tool (with approval gate)

**Deliverable:** AI chat blocks work with streaming responses, tool use, and multi-backend support.

---

## Phase 6: Remote Connections (~1,500 lines)

**Ports:** `remote` (1,226 LOC), `genconn` (457 LOC), `wsl` (211 LOC), `wslconn` (1,033 LOC)

### 6.1 SSH Client (`backend/remote/ssh.rs`, ~800 lines)

**New dependency:** `russh = "0.48"` (pure Rust SSH)

- `pub async fn connect(host: &str, config: SshConfig) -> Result<SshConnection, Error>`
- Key authentication, password authentication, agent forwarding
- Port forwarding
- SFTP file operations

### 6.2 Generic Connection Interface (`backend/remote/conn.rs`, ~200 lines)

Port `pkg/genconn/`:

- `Connection` trait abstracting local/SSH/WSL
- `pub fn exec_command(conn: &dyn Connection, cmd: &str) -> Result<Output, Error>`
- `pub fn open_shell(conn: &dyn Connection, size: TermSize) -> Result<ShellProc, Error>`

### 6.3 WSL Support (`backend/remote/wsl.rs`, ~400 lines)

Port `pkg/wsl/` + `pkg/wslconn/`:

- Windows-only (`#[cfg(target_os = "windows")]`)
- `pub fn list_distros() -> Result<Vec<String>, Error>`
- `pub fn connect_wsl(distro: &str) -> Result<WslConnection, Error>`
- Implements `Connection` trait

**Deliverable:** SSH and WSL connections work for remote terminal blocks.

---

## Phase 7: Utilities (~1,200 lines)

**Ports:** `ijson` (839 LOC), `vdom` (2,130 LOC), `suggestion` (758 LOC), misc

### 7.1 Incremental JSON (`backend/ijson.rs`, ~500 lines)

Port `pkg/ijson/`:

- Streaming JSON parser for large files
- Incremental updates (patch-based)
- Budget-based truncation

### 7.2 Virtual DOM Protocol (`backend/vdom/`, ~800 lines)

Port `pkg/vdom/`:

- VDom element types and rendering
- React-like hooks system
- HTML component serialization
- Only needed if AgentMux uses custom Wave Apps (Go SDK widgets)

### 7.3 Suggestion Engine (`backend/suggestion.rs`, ~400 lines)

Port `pkg/suggestion/`:

- File path walking and completion
- Suggestion ranking
- Context-aware completions

### 7.4 Minor Utilities

| Module | Lines | Priority |
|--------|-------|----------|
| `faviconcache.rs` | ~100 | Low — fetch and cache favicons |
| `trimquotes.rs` | ~30 | Trivial |
| `userinput.rs` | ~100 | Medium — user prompts |
| `wcloud.rs` | ~200 | Low — cloud features |
| `telemetry.rs` | ~300 | Low — can defer |

**Deliverable:** All utility functions available for feature parity.

---

## Phase 8: Server Entrypoint & Go Elimination (~500 lines)

### 8.1 Unified Server Startup (`backend/server/mod.rs`)

Replace `cmd/server/main-server.go`:

```rust
pub async fn start_backend(app: &AppHandle) -> Result<(), Error> {
    // 1. Init stores
    let wstore = WaveStore::open(&data_dir.join("waveobj.db"))?;
    let fstore = FileStore::open(&data_dir.join("filestore.db"))?;

    // 2. Run migrations
    // (done automatically by WaveStore::open)

    // 3. Ensure initial data
    wcore::ensure_initial_data(&wstore)?;

    // 4. Start services
    let router = RpcRouter::new();
    register_all_handlers(&router, &wstore, &fstore);

    // 5. Start WebSocket server (for wsh CLI compatibility)
    let ws_addr = find_free_port();
    tokio::spawn(run_ws_server(ws_addr, router.clone()));

    // 6. Start HTTP server
    let http_addr = find_free_port();
    tokio::spawn(run_http_server(http_addr, wstore.clone(), fstore.clone()));

    // 7. Start background tasks
    fstore.start_flusher();
    tokio::spawn(heartbeat_loop(data_dir));
    tokio::spawn(config_watcher(config_dir));

    // 8. Emit ready event to frontend
    app.emit("backend-ready", BackendReady { ws_addr, http_addr })?;

    Ok(())
}
```

### 8.2 Remove Go Sidecar

Once all phases pass integration tests:

1. Remove sidecar spawning from `src-tauri/src/sidecar.rs`
2. Remove `externalBin` from `tauri.conf.json`
3. Call `start_backend()` directly in Tauri's `setup()` hook
4. Remove Go binary build steps from CI
5. Keep `wsh` CLI as separate Go binary (can port later)

### 8.3 Dual-Mode Transition

During development, support both modes:

```rust
// src-tauri/src/lib.rs setup()
if cfg!(feature = "native-backend") {
    // Phase 8: run backend in-process
    backend::server::start_backend(&app).await?;
} else {
    // Current: spawn Go sidecar
    sidecar::spawn_backend(&app).await?;
}
```

**Deliverable:** Single binary. No Go process. Full feature parity.

---

## Go Package → Rust Module Mapping

| Go Package | Lines | Rust Module | Phase | Status |
|------------|-------|-------------|-------|--------|
| `waveobj` | 1,289 | `backend/waveobj.rs` | 1 | DONE |
| `wstore` | 779 | `backend/storage/wstore.rs` | 1 | DONE |
| `filestore` | 1,863 | `backend/storage/filestore.rs` | 1 | DONE |
| `wshrpc` (types) | 993 | `backend/rpc_types.rs` | 1 (partial) | DONE (partial) |
| `eventbus` | 87 | `backend/eventbus.rs` | 2 | TODO |
| `wps` | 338 | `backend/wps.rs` | 2 | TODO |
| `wconfig` | 1,102 | `backend/wconfig/` | 2 | TODO |
| `wcore` | 1,236 | `backend/wcore.rs` | 2 | TODO |
| `wshrpc` (full) | 993 | `backend/rpc/types.rs` | 3 | TODO |
| `wshutil` | 2,754 | `backend/rpc/router.rs` | 3 | TODO |
| `web` | 955 | `backend/server/` | 3 | TODO |
| `blockcontroller` | 1,806 | `backend/blockcontroller/` | 4 | TODO |
| `shellexec` | 871 | `backend/shellexec/` | 4 | TODO |
| `blocklogger` | 92 | `backend/blocklogger.rs` | 4 | TODO |
| `waveai` | 1,049 | `backend/ai/` | 5 | TODO |
| `aiusechat` | 2,478 | `backend/ai/usechat.rs` | 5 | TODO |
| `remote` | 1,226 | `backend/remote/` | 6 | TODO |
| `genconn` | 457 | `backend/remote/conn.rs` | 6 | TODO |
| `wsl` | 211 | `backend/remote/wsl.rs` | 6 | TODO |
| `wslconn` | 1,033 | `backend/remote/wsl.rs` | 6 | TODO |
| `ijson` | 839 | `backend/ijson.rs` | 7 | TODO |
| `vdom` | 2,130 | `backend/vdom/` | 7 | TODO |
| `suggestion` | 758 | `backend/suggestion.rs` | 7 | TODO |
| `service` | 458 | (merged into wcore) | 2 | TODO |
| `authkey` | 39 | (handled by Tauri auth) | — | N/A |
| `docsite` | 47 | (trivial, defer) | 7 | TODO |
| `schema` | 54 | (code gen, not runtime) | — | N/A |
| `gogen` | 117 | (code gen, not needed) | — | N/A |
| `tsgen` | 530 | (code gen, not needed) | — | N/A |
| `telemetry` | 408 | `backend/telemetry.rs` | 7 | TODO |
| `panichandler` | 43 | (Rust panic hook) | — | N/A |
| `trimquotes` | 31 | (inline) | — | N/A |
| `utilds` | 122 | (inline or stdlib) | — | N/A |
| `wavebase` | 430 | `backend/platform.rs` | 2 | TODO |
| `wcloud` | 306 | `backend/wcloud.rs` | 7 | TODO |
| `waveapp` | 751 | `backend/waveapp.rs` | 7 | TODO |
| `faviconcache` | 196 | `backend/faviconcache.rs` | 7 | TODO |
| `userinput` | 134 | `backend/userinput.rs` | 7 | TODO |
| `cmd/server` | ~700 | `backend/server/mod.rs` | 8 | TODO |

**Total Go LOC to port:** ~27,000 (excluding code gen, tests, and N/A packages)
**Already ported:** ~3,900 (Go equivalent LOC for Phase 1)
**Remaining:** ~23,000 Go LOC → estimated ~18,000-20,000 Rust lines

---

## New Cargo Dependencies by Phase

| Phase | Crate | Version | Purpose |
|-------|-------|---------|---------|
| 2 | `notify` | 7 | File system watching for config |
| 3 | `tokio-tungstenite` | 0.26 | WebSocket server |
| 3 | `axum` | 0.8 | HTTP server |
| 3 | `futures-util` | 0.3 | Stream utilities |
| 3 | `tower` | 0.5 | HTTP middleware (auth) |
| 3 | `tower-http` | 0.6 | CORS, compression |
| 4 | `portable-pty` | 0.8 | Cross-platform PTY |
| 5 | `reqwest` | 0.12 | HTTP client for AI APIs |
| 5 | `async-stream` | 0.3 | Async streaming |
| 6 | `russh` | 0.48 | SSH client |
| 6 | `russh-keys` | 0.48 | SSH key handling |

---

## Testing Strategy

Each phase includes in-memory tests with zero external dependencies:

| Phase | Test Focus | Approach |
|-------|-----------|----------|
| 2 | Event broadcasting, config read/write | In-memory, mock file system |
| 3 | RPC dispatch, WebSocket message round-trip | In-memory router, mock connections |
| 4 | Block lifecycle, PTY I/O | Mock PTY, in-memory stores |
| 5 | AI response streaming, tool execution | Mock HTTP, recorded responses |
| 6 | SSH connection, command execution | Mock SSH server (russh) |
| 7 | JSON incremental ops, VDOM serialization | Pure unit tests |
| 8 | Full server startup/shutdown | Integration test with real SQLite |

**Run all:** `cd src-tauri && cargo test backend`

**Note:** Full Tauri build requires system deps (glib-dev, webkit2gtk-dev) not available in the container. Use the standalone test crate at `/workspace/backend-test/` for CI in this environment.

---

## Conflict Avoidance with AgentA

AgentA owns the Tauri shell (Phases 0-12 of `specs/agentmux-tauri-migration.md`):
- `src-tauri/src/lib.rs`, `commands/`, `state.rs`, `sidecar.rs`, `menu.rs`, `tray.rs`

Agent3 owns the backend module:
- `src-tauri/src/backend/**` (isolated directory)

**Conflict surface:** 1-2 additive lines in `lib.rs` and `Cargo.toml` per phase. Git auto-resolves these.

---

## Recommended Execution Order

**Phase 2 (Core Services)** is the next PR. It unblocks Phases 3-4 which are the critical path to Go elimination.

```
Phase 2 → Phase 3 → Phase 4 → Phase 8 (MVP: terminals work without Go)
                  ↘ Phase 5 (AI, can parallel with 4)
                  ↘ Phase 6 (Remote, can parallel with 4-5)
                  ↘ Phase 7 (Utilities, as needed)
```

The **MVP milestone** (Phases 2-4 + 8) gives us local terminal blocks running natively in Rust — the most common use case. AI, remote connections, and utilities can follow incrementally.
