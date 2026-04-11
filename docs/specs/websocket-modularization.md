# Spec: websocket.rs Modularization

## Problem

`agentmuxsrv-rs/src/server/websocket.rs` is 2090 lines — a single file containing connection management, message routing, and all RPC command handlers. The monolithic `register_handlers()` function alone is 1590 lines.

## Current Structure

| Section | Lines | LOC | Concern |
|---------|-------|-----|---------|
| `handle_ws` + `handle_ws_connection` | 76-246 | 170 | WS upgrade, connection loop, heartbeat |
| `handle_incoming_text` | 251-446 | 195 | Message parsing, bus ops, RPC dispatch |
| Config/events handlers | 448-602, 1471-1526 | 200 | get/set config, event sub/unsub, metadata |
| Block/controller handlers | 644-891 | 250 | Resync, input, subprocess spawn, agent I/O |
| CLI resolution + auth | 898-1465 | 570 | Binary detection, npm install, auth checks, login |
| Forge handlers | 1530-2039 | 510 | Agents, content, skills, history, import CRUD |
| Utilities | 2044-2090 | 50 | cmd wrapper, version detection, input parsing |

## Target Structure

```
src/server/
├── websocket.rs              → mod.rs (80 LOC: handle_ws, register_handlers orchestration)
└── websocket/
    ├── mod.rs                 re-exports + register_all_handlers()
    ├── connection.rs          handle_ws_connection, heartbeat (170 LOC)
    ├── dispatch.rs            handle_incoming_text, bus ops (195 LOC)
    ├── cli_utils.rs           make_cli_cmd, get_cli_version, parse_block_input (50 LOC)
    └── handlers/
        ├── mod.rs             register_*() fns from each module
        ├── block.rs           controller resync/input, subprocess, agent I/O (250 LOC)
        ├── cli.rs             resolvecli, checkcliauth, runclilogin (570 LOC)
        ├── config.rs          config get/set, event sub/unsub, metadata, AI (200 LOC)
        └── forge.rs           agents/content/skills/history/import CRUD (510 LOC)
```

Each handler module exports a `register_<group>(router: &mut WshRpcRouter, state: Arc<AppState>)` function.

## Coupling Analysis

| Module | Dependencies | Split Feasibility |
|--------|-------------|-------------------|
| **cli.rs** | broker (logging only) | Easy — self-contained |
| **forge.rs** | wstore, broker | Easy — repetitive CRUD pattern |
| **config.rs** | config_watcher, wstore, broker | Easy — read-mostly |
| **block.rs** | blockcontroller, wstore, broker, reactive_handler | Medium — multiple subsystems |
| **dispatch.rs** | WSIncoming struct, all message types | Medium — routes to many concerns |
| **connection.rs** | AppState, event bus, RPC engine | Medium — core event demux |

## Implementation Order

1. **Extract cli.rs** (570 LOC, zero coupling to other handlers) — proves the pattern
2. **Extract forge.rs** (510 LOC, isolated CRUD) — bulk reduction
3. **Extract config.rs** (200 LOC) — straightforward
4. **Extract block.rs** (250 LOC) — needs blockcontroller plumbing
5. **Extract dispatch.rs + connection.rs** — last, hardest (core loop)

Each step: extract → verify `cargo check` → commit. Never move more than one group per commit.

## Shared Types

`WSIncoming` struct stays in `websocket/mod.rs` — all modules need it.

Handler registration pattern (each module):
```rust
pub fn register_cli_handlers(router: &mut WshRpcRouter, state: Arc<AppState>) {
    router.register("resolvecli", {
        let state = state.clone();
        move |_ctx, args| { /* ... */ }
    });
    // ...
}
```

## Public API

Only `handle_ws` is exported (`server/mod.rs` routes to it). Internal structure changes are invisible to the rest of the crate.

## Verification

After each extraction:
1. `cargo check -p agentmuxsrv-rs` — zero errors
2. `cargo test -p agentmuxsrv-rs` — tests pass
3. `task dev` — WebSocket commands still work (terminal, agent pane, forge)
