# SPEC: Agent App API — Identity, Presets & Memories Namespaces

> **Archived 2026-07-12.** Historical — `identity.*`/`preset.*`/`memory.*` App API shipped as specced (`preset.*` later renamed `bundle.*`). Consolidated tracking: issue #2024.

**Date:** 2026-06-27  
**Status:** Draft — open questions resolved  
**Author:** AgentX  
**Related:** `agentmux-srv/src/server/app_api.rs`, `agentmux-srv/src/server/agent_handlers.rs`, `agentmux-srv/src/server/identity_handlers.rs`, `agentmux-srv/src/server/native_memory_handlers.rs`

---

## 1. Problem

The Agent App API (`app_api.rs`) is the only RPC surface agents can reach (the permission boundary in `docs/internals/agent-app-api.md` is explicit about this). It currently covers agent lifecycle, panes, file I/O, and sessions — but has **no identity, preset, or memory surface**.

This means an agent cannot:
- Register or update its own GitHub/Anthropic/AWS credential so the Trust Center shows it as connected
- Read or modify the Memory bundle (preset) it was launched with
- Read or write its own native memory files (Claude Code's `$CLAUDE_CONFIG_DIR/projects/…/memory/*.md`)

Every one of those operations currently requires either direct DB manipulation or calling low-level handlers that agents aren't allowed to reach.

---

## 2. Goal

Add three new namespaces to the App API with the highest-value endpoints first. All operations go through the existing storage + event-bus layer, so the Trust Center UI updates reactively without a reload.

| Namespace | Surface |
|-----------|---------|
| `identity.*` | Credential accounts + agent-to-account links |
| `preset.*` | Memory bundles (what Trust Center calls "Presets") |
| `memory.*` | Native agent memory files (`$CLAUDE_CONFIG_DIR/projects/…/memory/*.md`) |

Security rule that threads through all three: **writes are agent-scoped** (an agent can only mutate its own records), **reads of shared catalogs are allowed** (list all presets), **enumeration of other agents' secrets is denied**.

---

## 3. Resolved Open Questions

### OQ1 — How do handlers know which agent is calling? (S1 enforcement)

`RpcContext` (`agentmux-srv/src/backend/rpc_types.rs:1345`) currently carries `client_type`, `blockid`, `tabid`, `conn` — no `agent_id`. The agent slug IS tracked on the WebSocket connection as `bus_agent_id` (set when the client sends `bus:register`), but it is not forwarded into `RpcContext`.

**Resolution:** Add an `agent_id: String` field to `RpcContext`. Populate it in `websocket.rs` when `set_rpc_context` is called, using the connection's `bus_agent_id` if available. App API handlers enforce S1 by comparing `ctx.agent_id` against the `agent_id` field in the request — they are equal or the call is rejected.

This is a one-line struct change + one assignment at the `set_rpc_context` call site. No wire-format break since the field is `skip_serializing_if = "String::is_empty"`.

### OQ2 — What is the memory storage backend?

`native_memory_handlers.rs` operates on **files**, not a key/value store. Specifically:

- Directory: `$CLAUDE_CONFIG_DIR/projects/<sanitized-cwd>/memory/`
- Files: `*.md` (max 10 MiB each, atomic tmp→rename write)
- Existing commands: `agent:memory:list`, `agent:memory:read_file`, `agent:memory:write_file`

**`CLAUDE_CONFIG_DIR` isolation:** AgentMux sets `CLAUDE_CONFIG_DIR` at agent spawn time (`app_api.rs`) so that Claude Code never writes to the global `~/.claude/`. The actual root is `~/.agentmux/shared/providers/claude/` for default agents and `~/.agentmux/shared/identities/<bundle_id>/claude/` for per-identity bundles. `native_memory_handlers.rs` must resolve this path from the agent's stored env blob (`agent_content_get(agent_id, "env")` → parse `CLAUDE_CONFIG_DIR`) rather than hardcoding `~/.claude/projects`.

The `memory.*` App API commands delegate directly to these handlers — no new storage layer. The only additions are: (a) routing through the App API permission boundary, and (b) emitting a `agent:memory:changed:<agent_id>` event after writes.

### OQ3 — Should `agent:memory:changed` be persisted?

The existing `agent:memory:write_file` returns `Ok(None)` — no event fired. Memory file reads are cheap (small `.md` files), so the event is **fire-and-forget** (`persist=0`). Any subscriber that missed the event can simply re-list to get current state.

---

## 4. Scope

### In scope (this spec)

- All commands in §5
- `RpcContext` extension (§3 OQ1)
- WPS events each command fires (§6)
- Mapping to existing low-level handlers (§7)
- Security invariants (§8)
- Handler registration in `register_app_api_handlers`

### Out of scope

- OAuth flows (`auth.start` / `auth.poll`) — these remain in `identity_handlers.rs` and are not elevated to the App API yet; they require a TTY/browser handoff that doesn't fit the agent-call model
- Bundle-level identity binding (`bindidentityaccount`) — bundle management stays user-facing; agents manage their own account links via `identity.account.upsert`
- Cross-agent memory reads — an agent reads only its own memory directory

---

## 5. API Surface

### 5.1 `identity.*` — Credentials

#### `identity.self.accounts`

List accounts currently linked to the calling agent.

```jsonc
// Request
{ "agent_id": "agentx" }

// Response
{
  "accounts": [
    {
      "account_id": "15f7fe0a-...",
      "provider":   "github",
      "name":       "agentx-github",
      "kind":       "api_key",      // "api_key" | "oauth"
      "status":     "valid",        // "valid" | "expired" | "unknown"
      "masked_tail": "w528",        // last 4 chars of secret — never the secret itself
      "updated_at": 1751020800000
    }
  ]
}
```

Delegates to: `listagentidentities { agent_id }` → resolves account records via `getidentityaccount` per entry.

---

#### `identity.account.upsert`

Create or update a credential account **and** link it to the calling agent for the given provider. Stores the secret in the OS keychain (never in SQLite). If `account_id` is omitted a new UUID is minted. If the agent already has an account linked for this provider, the existing link is replaced.

```jsonc
// Request
{
  "agent_id":   "agentx",          // must match ctx.agent_id (S1)
  "provider":   "github",
  "name":       "agentx-github",
  "kind":       "api_key",
  "secret":     "ghp_...",         // required for new account or key rotation
  "validate":   true,              // probe the provider to confirm credential works
  "account_id": "15f7fe0a-..."    // optional — omit to mint a new UUID
}

// Response
{
  "account_id":  "15f7fe0a-...",
  "provider":    "github",
  "name":        "agentx-github",
  "status":      "valid",
  "masked_tail": "w528",
  "valid":       true,
  "error":       null              // set if validate=true and probe failed
}
```

Delegates (in order):
1. `account.key.verify` — stores the secret in the keychain unconditionally, then probes the provider if `validate: true`. The `validate` field in the request controls only whether the probe is attempted — not whether the keychain write happens. If `validate: false` (or omitted), the secret is stored and the response `status` is `"unknown"`. (Step 1 is non-atomic with respect to steps 2–3; see compensation note below.)
2. If the agent already has a link for `provider`: call `unlinkagentidentity { agent_id, provider }` to remove it (the App API handler must do this explicitly — `linkagentidentity` inserts, it does not replace)
3. `linkagentidentity { agent_id, account_id, provider }` — creates the new link

**Compensation:** If step 3 fails (e.g. DB write error), the secret is already in the OS keychain with no agent link. The handler MUST attempt to delete the orphaned keychain entry via the existing keychain-delete path. If that also fails, log a warning and return an error to the caller — the caller can retry, which will re-enter at step 1 and overwrite the orphaned entry. This is the same best-effort-cleanup pattern used by other two-phase writes in `identity_handlers.rs`.

Fires: `identityaccounts:changed` (global) + `agentidentities:changed:<agent_id>`

---

#### `identity.account.validate`

Probe an existing stored account (by `account_id`) or an ad-hoc secret without storing anything.

```jsonc
// Request — probe stored account:
{ "account_id": "15f7fe0a-..." }

// Request — probe ad-hoc (not stored):
{ "provider": "github", "secret": "ghp_..." }

// Response
{
  "valid":       false,
  "status":      "expired",
  "error":       "HTTP 401: Bad credentials",
  "masked_tail": "w528"
}
```

Delegates to: `account.key.verify { validate: true }`. Read-only — fires no events.

---

#### `identity.self.unlink`

Remove the agent's link to a provider account. The account record stays in the DB.

```jsonc
// Request  { "agent_id": "agentx", "provider": "github" }  // agent_id must match ctx.agent_id (S1)
// Response { "unlinked": true }
```

Delegates to: `unlinkagentidentity { agent_id, provider }`

Fires: `agentidentities:changed:<agent_id>`

---

### 5.2 `preset.*` — Memory Bundles

The Trust Center calls these "Presets". Backend table: `db_memory_bundles`. The blank singleton (`is_blank: true`) cannot be mutated or deleted.

#### `preset.list`

List all presets — summary fields only, no instruction/context blobs.

```jsonc
// Request  {} (no params)
// Response
{
  "presets": [
    {
      "id":          "abc-123",
      "name":        "AgentX Default",
      "provider":    "claude",
      "model":       "claude-sonnet-4-6",
      "description": "AgentX's standard preset",
      "is_blank":    false,
      "updated_at":  1751020800000
    }
  ]
}
```

Delegates to: `listmemories`

---

#### `preset.get`

Fetch a full preset (instructions, context_files, mcp_servers, skills) by `id` or `name`.

```jsonc
// Request  { "id": "abc-123" }   OR   { "name": "AgentX Default" }
// Response — full Memory object (see db_memory_bundles schema in memory.md)
```

Delegates to: `getmemory { id }`. For `name` lookup: list + filter client-side (no name index on the table). Preset names are not enforced unique — if multiple presets share a name, return the one with the most recent `updated_at`. If zero match, return a not-found error.

---

#### `preset.upsert`

Create or update a preset. Omit `id` to create. Guards blank singleton.

```jsonc
// Request
{
  "id":            "abc-123",        // omit to create
  "name":          "AgentX Default",
  "provider":      "claude",
  "model":         "claude-sonnet-4-6",
  "description":   "...",
  "instructions":  "You are AgentX ...",
  "context_files": [],
  "mcp_servers":   [],
  "skills":        []
}
// Response — full updated Memory object
```

Delegates to: `upsertmemory`

Fires: `memories:changed`

---

#### `preset.delete`

Delete a preset by id. Rejects blank singleton and `seed-` prefixed bundles (see S4).

```jsonc
// Request  { "id": "abc-123" }
// Response { "deleted": true }
```

Delegates to: `deletememory { id }`. The handler must catch `StoreError::Other` from `bundle_memory_delete` and re-surface it as `FORBIDDEN: cannot delete a seeded bundle` (not a raw storage error string).

Fires: `memories:changed`

---

#### `preset.self.get`

Get the preset bound to the calling agent's current instance. Resolves via `memory_id` on `db_agent_instances`; returns the blank singleton object (not null) if `memory_id` is unset.

```jsonc
// Request  { "agent_id": "agentx" }
// Response — full Memory object (is_blank: true when no preset is bound)
```

Delegates via: `instance_get_by_name(agent_id)` (storage layer — maps agent slug → `AgentInstance`) → read `memory_id` from the returned row → `getmemory { id: memory_id }`. `CommandGetAgentInstanceData` takes an instance UUID (`id: String`), not a slug, so the handler cannot call `getagentinstance` directly with the request's `agent_id`; `instance_get_by_name` is the correct intermediate. If `memory_id` is null, fall back to two steps: (1) `listmemories` + filter `is_blank: true` to retrieve the blank singleton's `id` (summary fields only — `listmemories` does not return `instructions`, `context_files`, `mcp_servers`, or `skills`), then (2) `getmemory { id: <blank_id> }` to fetch the full Memory object. Step 2 is required to satisfy the response contract — step 1 alone is insufficient.

---

### 5.3 `memory.*` — Native Agent Memories

These delegate directly to the existing `agent:memory:*` handlers in `native_memory_handlers.rs`. The storage is Claude Code's native memory directory: `$CLAUDE_CONFIG_DIR/projects/<sanitized-cwd>/memory/*.md` — see OQ2 for isolation details. All three commands already exist at the low-level RPC layer — these wrappers add (a) App API permission enforcement and (b) the `agent:memory:changed` event.

#### `memory.list`

List all memory files for the calling agent.

```jsonc
// Request  { "agent_id": "agentx" }
// Response
{
  "files": [
    {
      "filename":      "user_preferences.md",
      "is_index":      false,
      "metadata_type": "feedback",   // value of `metadata.type` YAML frontmatter key; null if absent or unparseable
      "size_bytes":    412,
      "modified_at":   1751020800000
    }
  ]
}
```

Delegates to: `agent:memory:list { agent_id }`

---

#### `memory.read`

Read one memory file by filename. Filenames must be `[a-zA-Z0-9_-]+.md`.

```jsonc
// Request  { "agent_id": "agentx", "filename": "user_preferences.md" }
// Response { "content": "---\nmetadata:\n  type: feedback\n---\n..." }
```

Delegates to: `agent:memory:read_file { agent_id, filename }`

---

#### `memory.write`

Write (create or replace) a memory file atomically. Max 10 MiB. Filename rules same as `memory.read`.

```jsonc
// Request
{
  "agent_id": "agentx",
  "filename": "user_preferences.md",
  "content":  "---\nmetadata:\n  type: feedback\n---\nUser prefers terse responses."
}
// Response  {} (empty on success)
```

Delegates to: `agent:memory:write_file { agent_id, filename, content }`

Fires: `agent:memory:changed:<agent_id>` (fire-and-forget, persist=0) — emitted only after the write succeeds. No event is fired on error.

---

## 6. WPS Events

| Command | Event | Scope | Persist |
|---------|-------|-------|---------|
| `identity.account.upsert` | `identityaccounts:changed` | global | 0 |
| `identity.account.upsert` | `agentidentities:changed:<agent_id>` | scoped | 0 |
| `identity.self.unlink` | `agentidentities:changed:<agent_id>` | scoped | 0 |
| `preset.upsert` | `memories:changed` | global | 0 |
| `preset.delete` | `memories:changed` | global | 0 |
| `memory.write` | `agent:memory:changed:<agent_id>` | scoped | 0 |

All events are `persist=0` (fire-and-forget), matching the existing pattern in `agent_handlers.rs`. Subscribers that miss an event due to reconnect must re-poll: `identity.self.accounts` for identity events, `preset.list` for preset events, `memory.list` for memory events.

`agent:memory:changed:<agent_id>` is a new event type. Subscribe via `eventsub` with scope `<agent_id>` to receive live updates when memory files change.

---

## 7. Implementation Map

| App API command | Existing handler(s) | New logic |
|-----------------|--------------------|-----------| 
| `identity.self.accounts` | `listagentidentities` + `getidentityaccount` per entry | Join results, mask secret tail |
| `identity.account.upsert` | `account.key.verify` → `unlinkagentidentity` (if existing link) → `linkagentidentity` | Sequential, S1 scope check, explicit unlink-then-relink for provider replacement |
| `identity.account.validate` | `account.key.verify { validate: true }` | Ad-hoc path skips write |
| `identity.self.unlink` | `unlinkagentidentity` | Direct delegation |
| `preset.list` | `listmemories` | Strip instruction/context blobs from response |
| `preset.get` | `getmemory` | Add name-based lookup via list+filter |
| `preset.upsert` | `upsertmemory` | Guard blank singleton; strip `is_global` and `sort_order` from request before delegating (S4a) |
| `preset.delete` | `deletememory` | Guard blank singleton + `seed-` prefix; catch `StoreError::Other` and re-surface as `FORBIDDEN` |
| `preset.self.get` | `instance_get_by_name(agent_id)` → `getmemory` | `instance_get_by_name` maps agent slug → instance row → `memory_id`; `getagentinstance` takes a UUID and cannot be used directly |
| `memory.list` | `agent:memory:list` | S1 scope check, emit no event |
| `memory.read` | `agent:memory:read_file` | S1 scope check |
| `memory.write` | `agent:memory:write_file` | S1 scope check, emit `agent:memory:changed` |

All handlers register in `register_app_api_handlers` in `app_api.rs`.

---

## 8. Security Invariants

**S1 — Agent-scoped writes only.** For any command with an `agent_id` field, the handler MUST:
1. Reject with `FORBIDDEN: unauthenticated agent connection` if `ctx.agent_id` is empty (i.e., the connection never sent `bus:register`; `unwrap_or_default()` in §9 leaves it `""`).
2. Reject with `FORBIDDEN: agent_id mismatch` if `ctx.agent_id != request.agent_id`.

Both checks are required. Checking only for equality is insufficient because `ctx.agent_id = ""` and `request.agent_id = ""` would pass the equality check, allowing an unauthenticated connection to call any agent-scoped command with an empty agent_id. This requires the one-time `RpcContext` extension described in §3 OQ1.

**S2 — No secret enumeration.** `identity.self.accounts` returns `masked_tail` (last 4 chars), never the plaintext secret. Plaintext lives only in the OS keychain.

**S3 — No cross-agent memory reads.** `memory.*` commands reject `agent_id` values that differ from `ctx.agent_id`. The memory directory path is derived from the agent's `working_directory` in `db_agent_definitions` — an agent cannot path-traverse into another agent's directory.

**S4 — Blank preset and seeded-bundle guard.** `preset.upsert` and `preset.delete` return `FORBIDDEN: cannot mutate the blank preset` when the target id is the `is_blank` singleton. `preset.delete` additionally returns `FORBIDDEN: cannot delete a seeded bundle` when the target id starts with `seed-` — `bundle_memory_delete` rejects these at the storage layer (`memory_bundles.rs:194`); the App API wrapper must catch the `StoreError::Other` and re-surface it as a clean `FORBIDDEN` rather than a raw storage error string.

**S4a — `is_global` and `sort_order` strip.** `preset.upsert` MUST strip `is_global` and `sort_order` from the caller-supplied payload before delegating to `upsertmemory`. `bundle_memory_upsert` passes `is_global = excluded.is_global` straight through (`memory_bundles.rs:156`), so an agent sending `"is_global": true` would elevate its preset to global-brain status, injecting its instructions into every other agent's context at launch. These fields are not part of the `preset.upsert` API surface and must be silently ignored (not rejected) to allow future callers to pass extra fields without breaking.

**S5 — validate=true is minimally destructive.** `identity.account.validate` never writes a new keychain entry and never creates or deletes DB rows. The only permitted DB side-effect is updating the `status` column on an existing account row (when `account_id` is supplied and the probe result resolves a previously-`unknown` status). No write occurs for ad-hoc probes (no `account_id`).

**S6 — Memory filename validation.** `memory.read` and `memory.write` enforce the existing `validate_filename` check from `native_memory_handlers.rs`: stem must be `[a-zA-Z0-9_-]+`, extension must be `.md`, no path separators, max 200-char stem. This blocks path traversal at the App API layer before the low-level handler can be reached.

**Error format.** Security rejections return a string error in the existing RPC error convention: `"FORBIDDEN: <reason>"` (e.g. `"FORBIDDEN: agent_id mismatch"`, `"FORBIDDEN: cannot mutate the blank preset"`). This matches the string-error pattern used by other handlers in `app_api.rs` — no new error type is introduced.

---

## 9. RpcContext Extension (prerequisite)

`agentmux-srv/src/backend/rpc_types.rs` — add one field to `RpcContext`:

```rust
pub struct RpcContext {
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "ctype")]
    pub client_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tabid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub conn: String,
    // NEW: slug of the authenticated agent (from bus:register), empty for non-agent connections
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
}
```

`websocket.rs` — populate it when calling `set_rpc_context`:

```rust
engine.set_rpc_context(RpcContext {
    client_type: ...,
    blockid: block_id.clone(),
    tabid: tab_id.clone(),
    conn: String::new(),
    agent_id: bus_agent_id.clone().unwrap_or_default(), // NEW
});
```

No wire-format break: `skip_serializing_if = "String::is_empty"` means existing clients that don't send `agent_id` get an empty string, which the S1 check treats as "unauthenticated agent connection → reject agent-scoped write commands".

---

## 10. Phasing

| Phase | Commands | Why first |
|-------|----------|-----------|
| **P0** | `RpcContext` extension | Prerequisite for S1 — must land before any identity/memory writes. The change is a single struct field + one assignment; it can be bundled in the same PR as P1 rather than shipped as a bare prereq commit. |
| **P1** | `identity.account.upsert`, `identity.self.accounts`, `identity.account.validate` | Unblocks agents registering their own credentials |
| **P2** | `identity.self.unlink`, `preset.list`, `preset.get`, `preset.upsert`, `preset.delete`, `preset.self.get` | Preset management — agents that self-configure |
| **P3** | `memory.list`, `memory.read`, `memory.write` | Thin wrappers over existing handlers; low risk, high value for memory-aware agents |
