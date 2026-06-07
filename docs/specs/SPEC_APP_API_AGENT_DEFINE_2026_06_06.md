# SPEC: App API — `agent.define` (Import / Upsert Agent Definition)

**Status:** Draft  
**Date:** 2026-06-06  
**Author:** AgentA  
**Tier:** 1 extension (same auth + transport as existing `agent.open`)  
**Tracking:** Discussion #1205 (app API long-term thread)  
**Related:** `docs/specs/app-api-extension.md`, `docs/specs/app-api-status.md`

---

## 1. Purpose

Allow any process — including a Claude agent running inside an AgentMux pane — to **create
or update an agent definition** via the HTTP/WebSocket app API, without requiring:
- direct SQLite access, or
- an application restart for the definition to become visible.

The endpoint completes the "agent-driven self-configuration" loop:

```
Agent reads  AGENTMUX_LOCAL_URL + AGENTMUX_AUTH_KEY
         ↓
POST /agentmux/service  {"service":"agent","method":"define","args":[{...}]}
         ↓
Sidecar inserts/updates db_agent_definitions
         ↓
Broadcast agents:changed  →  all frontends refresh My Agents instantly
```

This is the **robust, restart-free alternative** to `scripts/import-agents.sh`.

---

## 2. Transport & Auth (unchanged from existing Tier 1)

| Property | Value |
|---|---|
| HTTP endpoint | `POST http://<AGENTMUX_LOCAL_URL>/agentmux/service` |
| WebSocket endpoint | `ws://<AGENTMUX_LOCAL_URL>/ws?authkey=<token>` |
| Auth header (HTTP) | `X-AuthKey: <token>` |
| Auth query param (WS) | `authkey=<token>` |
| Token source | `AGENTMUX_AUTH_KEY` env var (injected at agent spawn time) |

The token is stripped from the sidecar's own environment at startup and injected only into
agent subprocesses — it never leaks to child processes that agents spawn.

**HTTP envelope** (`WebCallType` — matches all other `/agentmux/service` calls):
```json
{ "service": "agent", "method": "define", "args": [{ ... }] }
```

**WebSocket envelope** (`WshRpc` — for the WebSocket `/ws` path):
```json
{ "command": "agent.define", "reqid": "<uuid-v4>", "data": { ... } }
```

Both routes call the same `agent_define_core` handler and return equivalent results.

---

## 3. Request Shape

```typescript
interface AgentDefineRequest {
  // Required
  name: string;                    // Display name ("Maks", "Senior Dev")

  // Identity — one of these must be set
  provider?: string;               // "claude" | "openai" | "gemini" | "codex" | ...
  model?: string;                  // e.g. "claude-sonnet-4-6" (provider inferred if omitted)

  // Optional configuration — all fields mirror db_agent_definitions columns
  system_prompt?: string;          // System/instruction text
  working_directory?: string;      // Absolute path; "" = auto-allocate
  shell?: string;                  // Shell override, e.g. "/bin/bash"
  icon?: string;                   // Emoji or short string used as avatar
  env?: Record<string, string>;    // Extra env vars to inject at agent spawn

  // Import behaviour
  if_exists?: "skip" | "update" | "error";   // Default: "skip"
  //   "skip"   — if a definition with the same slug already exists, return its id; no update
  //   "update" — overwrite all provided fields on the existing definition
  //   "error"  — return error if a definition with the same slug exists

  // Definition metadata
  is_seeded?: 0 | 1;               // Default: 0 (user-owned, appears in My Agents)
  parent_id?: string;              // id of the definition this was forked from

  // Instance stub — controls whether a My Agents row appears immediately
  create_instance_stub?: boolean;  // Default: true
  //   true  → insert a stopped AgentInstance row so the agent appears in My Agents right away
  //   false → definition is created but won't appear in My Agents until first launch
}
```

### 3.1 Slug derivation

The backend derives a **slug** from `name` to detect duplicates across calls:
- Lowercase, whitespace → `_`, strip non-`[a-z0-9_]` chars
- Examples: `"Maks"` → `"maks"`, `"Senior Dev"` → `"senior_dev"`
- Slug uniqueness is checked across all user-owned (`is_seeded=0`) definitions only

---

## 4. Response Shape

**Success (HTTP 200):**

```typescript
interface AgentDefineResponse {
  ok: true;
  definition_id: string;
  slug: string;
  action: "created" | "updated" | "skipped";  // What actually happened
  instance_stub_id?: string;                   // Set when create_instance_stub=true + action=created
}
```

**Error (HTTP 200 with error envelope, following existing Tier 1 pattern):**

```typescript
interface AgentDefineError {
  ok: false;
  error: string;
  code:
    | "UNKNOWN_PROVIDER"    // provider string not in the registered provider set
    | "MISSING_IDENTITY"    // neither provider nor model was given
    | "MISSING_NAME"        // name was empty or whitespace-only
    | "ALREADY_EXISTS"      // if_exists="error" and slug already registered
    | "INVALID_FIELD"       // a field value failed validation (details in error string)
    | "INTERNAL";
}
```

---

## 5. Behaviour

### 5.1 On `action = "created"` (new definition)

1. Generate a new UUID for `definition_id`.
2. Insert row into `db_agent_definitions` with all supplied fields; fill defaults:
   - `is_seeded = 0`
   - `created_at = now_ms()`
   - `working_directory = ""` if not supplied (app allocates on first launch)
3. If `create_instance_stub = true` (default): insert a stub `db_agent_instances` row:
   - `status = "stopped"`
   - `instance_name = <name>`
   - `display_hidden = 0`
   - `definition_id = <new_definition_id>`
   - `started_at = now_ms()`, `ended_at = 0`
4. Broadcast `agents:changed` to all connected WebSocket clients.
   - All open `AgentPicker` frontends refresh `useAgentDefinitions()` and `MyAgentsList`
     immediately — no restart required.
5. Return `AgentDefineResponse { action: "created", ... }`.

### 5.2 On `action = "updated"` (`if_exists="update"`, slug found)

1. Look up existing definition by slug.
2. Update only the fields present in the request (treat missing fields as "no change").
3. Broadcast `agents:changed`.
4. Return `AgentDefineResponse { action: "updated", definition_id: existing_id }`.

### 5.3 On `action = "skipped"` (`if_exists="skip"`, slug found)

1. Look up existing definition by slug.
2. No DB write.
3. No broadcast.
4. Return `AgentDefineResponse { action: "skipped", definition_id: existing_id }`.

### 5.4 `agents:changed` broadcast

The broadcast is the same message the backend already emits on `AgentDefinitionForkCommand`
and other definition mutations. The frontend's `useAgentDefinitions()` hook — already
subscribed — will re-fetch and update signals. No frontend changes needed.

---

## 6. Example — Agent calling from inside a pane

A Claude agent running in an AgentMux pane can drive this via `curl` (already available
in the pane's shell):

```bash
#!/usr/bin/env bash
# Create a new agent definition and see it appear in My Agents immediately.
# The HTTP endpoint uses the WebCallType envelope: service + method + args[].

curl -s -X POST "$AGENTMUX_LOCAL_URL/agentmux/service" \
  -H "Content-Type: application/json" \
  -H "X-AuthKey: $AGENTMUX_AUTH_KEY" \
  -d '{
    "service": "agent",
    "method": "define",
    "args": [{
      "name": "Refactor Agent",
      "provider": "claude",
      "working_directory": "/c/Systems/agentmux",
      "if_exists": "skip"
    }]
  }'
```

Expected response:
```json
{
  "success": true,
  "data": {
    "definition_id": "a1b2c3d4-...",
    "slug": "refactor_agent",
    "action": "created",
    "instance_stub_id": "si-a1b2c3d4..."
  }
}
```

> **Note on envelope format:** The HTTP `/agentmux/service` endpoint uses the
> `WebCallType` envelope (`service` + `method` + `args[]`), which matches all other
> AgentMux HTTP service calls. The WebSocket path uses the `WshRpc` envelope
> (`command` + `reqid` + `data`). Both routes call the same `agent_define_core`
> handler and return equivalent results.

The agent appears in My Agents on all open frontends within ~100ms (broadcast roundtrip),
without any restart.

---

## 7. `amux` CLI integration (when amux ships)

```bash
# Idiomatic CLI form (amux — planned, see app-api-status.md)
amux agent define \
  --name "Refactor Agent" \
  --provider claude \
  --model claude-sonnet-4-6 \
  --system-prompt "You are a senior Rust engineer." \
  --workdir /c/Systems/agentmux \
  --if-exists skip

# Batch import from a JSON file
amux agent define --file agents.json
```

The `amux` CLI is the long-term preferred interface; `curl` is the stable fallback while
`amux` is in development.

---

## 8. Batch import (single call, multiple definitions)

To avoid multiple round-trips when importing many agents, the endpoint accepts an array:

```typescript
// Alternative request shape: array at top level
type AgentDefineBatchRequest = AgentDefineRequest[];

// Response when batch:
interface AgentDefineBatchResponse {
  ok: true;
  results: Array<AgentDefineResponse | AgentDefineError>;
  // agents:changed broadcast fires ONCE after all inserts (not per-item)
}
```

The endpoint detects array vs object by checking the `data` field type. A single
`agents:changed` broadcast covers all items in the batch.

---

## 9. Backend implementation — key files

| File | Change |
|---|---|
| `agentmux-srv/src/server/app_api.rs` | Add `handle_agent_define()` function; register under `COMMAND_AGENT_DEFINE` |
| `agentmux-srv/src/backend/rpc_types.rs` | Add `pub const COMMAND_AGENT_DEFINE: &str = "agent.define";` |
| `agentmux-srv/src/backend/storage/agents.rs` | Add `upsert_agent_definition(conn, data, if_exists) -> Result<AgentDefineResult>` |
| `frontend/types/gotypes.d.ts` | No change needed (response is JSON, not a typed frontend RPC) |

### 9.1 Slug uniqueness query

```sql
SELECT id FROM db_agent_definitions
WHERE lower(replace(name, ' ', '_')) = ?
  AND is_seeded = 0
LIMIT 1;
```

### 9.2 Instance stub insert (mirrors import-agents.sh)

```rust
conn.execute(
    "INSERT OR IGNORE INTO db_agent_instances
       (id, definition_id, parent_instance_id, block_id, session_id,
        status, github_context, identity_id, memory_id, instance_name,
        working_directory, display_hidden, started_at, ended_at, created_at)
     VALUES
       (?1, ?2, '', '', '', 'stopped', '', '', '', ?3, '', 0, ?4, 0, ?4)",
    params![stub_id, definition_id, name, now_ms],
)?;
```

### 9.3 Broadcast

Re-use the existing broadcast helper already called by `AgentDefinitionForkCommand`:
```rust
broadcast_agents_changed(&state).await;
```

---

## 10. Fit within the Tier taxonomy

From `docs/specs/app-api-extension.md`:

| Tier | Scope | This endpoint |
|---|---|---|
| 1 | Core agent/pane lifecycle over existing RPC | ✅ Yes — definition management is agent lifecycle |
| 2 | File/editor integration | No |
| 3 | UI state (tab, focus, layout) | No |
| 4 | Auth, identity, secrets | No |
| 5 | MCP / plugin host | No |

`agent.define` is a natural Tier 1 extension alongside `agent.open`. It completes the
create-before-open flow: an agent can call `agent.define` to register itself, then
`agent.open` to spawn a fresh instance of the definition in a new pane.

---

## 11. Security constraints (unchanged from existing Tier 1)

- Requires valid `X-AuthKey` — not accessible to processes outside AgentMux.
- Only operates on user-owned definitions (`is_seeded=0`). Seeded templates cannot
  be created or modified via this endpoint.
- `working_directory` is stored as-is; validation (path-traversal, sandbox) is the same
  as `agent.open` (existing behaviour).
- No rate limit beyond what the RPC engine already applies.

---

## 12. Open questions

| # | Question | Default |
|---|----------|---------|
| 1 | Should `update` action create a stub instance if one doesn't exist yet? | Yes — same as `created` |
| 2 | Should `name` be the uniqueness key, or should callers pass an explicit `slug`? | `name`-derived slug; caller can pass `slug` override |
| 3 | Should the batch form accept a NDJSON stream for very large imports? | Out of scope for v1 |
| 4 | Should `agent.define` also trigger a one-time allocation of `working_directory` if ""? | No — allocate lazily at `agent.open` time (existing behaviour) |
