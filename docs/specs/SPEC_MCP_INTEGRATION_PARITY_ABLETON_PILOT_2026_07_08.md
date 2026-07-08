# Spec: MCP integration parity with Claude Desktop / Cursor, piloted on Ableton MCP

**Date:** 2026-07-08
**Author:** Agent1
**Status:** Proposal
**Related:** `agentmux-srv/src/server/app_api/mcp.rs`, `agentmux-srv/src/backend/storage/mcp_servers.rs`, `agentmux-srv/src/backend/agent_config.rs`, `frontend/app/view/mcp/`, `frontend/app/view/agent/agent-mcp-model.ts`, `frontend/app/store/toolchain-capabilities.ts`, `frontend/app/view/accounts/oauth-catalog.ts`
**Governing context:** `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` (MCP Server is a first-class composable-model primitive as of Phase 1, PR #1877), `docs/specs/EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md` (Armory IA, deferred **Policy** primitive), `docs/specs/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` (Armory umbrella)

---

## 1. Summary

AgentMux's MCP Server primitive today is a **name + transport string + opaque JSON blob**, hand-typed by the user and merged verbatim into `.mcp.json`. That covers the mechanical case (an already-correct JSON snippet gets to the agent) but covers none of what makes Claude Desktop's and Cursor's MCP integrations *usable*: structured config with variable interpolation, OAuth for remote servers, a health/prerequisite check before the agent ever calls a tool, tool-level introspection and approval, and a catalog/one-click-install flow instead of "paste JSON here."

This spec (a) inventories that gap against Claude Desktop and Cursor's current (mid-2026) feature sets, (b) proposes an incremental design that extends the existing `McpServer` primitive rather than replacing it, and (c) uses **[ahujasid/ableton-mcp](https://github.com/ahujasid/ableton-mcp)** as the first real-world integration to build and validate against, because it exercises the single gap that matters most and is currently invisible: an **external, AgentMux-uncontrollable prerequisite** (Ableton Live running, with a manually-installed Remote Script selected as the active Control Surface) that today fails silently as a hung or cryptic tool call three turns into a conversation, instead of a clear, actionable status in the Armory before the agent ever tries.

---

## 2. Current state (as of `a9eb7da9`, 2026-07-08)

The v1 composable-model `McpServer` primitive (Phase 1 of the Preset→Bundle refactor, already shipped):

- **Storage** (`mcp_servers.rs`, `migrations.rs:416-441`): `db_mcp_servers(id, name, transport TEXT DEFAULT 'stdio', config TEXT DEFAULT '{}', is_global, created_at, updated_at)` + `db_agent_mcp_ref(agent_id, mcp_id)` bind table. `config` is an opaque JSON string — the store never looks inside it.
- **App API** (`app_api/mcp.rs`): `mcp.list/get/upsert/delete/bind/unbind` (agent-scoped, `check_s1`-gated) + `mcp.catalog.list/upsert/delete` (window-scoped, global rows only — the Armory's **MCP Servers** tab, `armory-view.tsx:20`).
- **Launch-time merge** (`agent_config.rs:320` `build_mcp_config_from_refs`): synthesizes the reserved `agentmux` entry, layers the legacy blob, then layers each bound `McpServer.config` **as-is** under `mcpServers.<name>` in `.mcp.json`. Whatever JSON the user typed is whatever Claude Code receives — AgentMux never validates it beyond "is it a JSON object."
- **UI** (`AgentMcpModal.tsx`, `mcp-manager.tsx`): a name field, a free-text transport field (defaults `"stdio"`, not validated against known values), and a raw JSON `<textarea>` for `config`. No connect test, no tool list, no distinction between `stdio`/`sse`/`streamable-http` beyond what the user happens to type inside the blob.

**What this means concretely:** adding a server that needs an env var means pasting a literal secret into the textarea (stored in SQLite in plaintext); adding a remote OAuth-authenticated server means the user must obtain a token out-of-band and paste it into a `headers` object by hand; and there is **no feedback loop** — `mcp.upsert` succeeds as long as the JSON parses, whether or not the server is real, reachable, or ever starts. `toolchain-capabilities.ts` already solved exactly this "is X actually available right now" problem for CLI toolchains (path probe + liveness probe, cached, shared, pollable) — nothing analogous exists for MCP servers.

---

## 3. Feature inventory: Claude Desktop / Cursor vs. AgentMux today

| Capability | Claude Desktop (2026) | Cursor (2026) | AgentMux today | Gap |
|---|---|---|---|---|
| Structured stdio config (command/args/env) | Yes (`.mcpb`/DXT manifest or manual JSON) | Yes, `mcpServers.<name>.{command,args,env}` | Raw JSON blob only — same shape is *possible* but the UI never structures it | UI/validation gap |
| Remote transport (Streamable HTTP) | Yes — Custom Connectors, MCP protocol 2025-11-25 | Yes — `url` + `headers` | Possible via raw JSON (`transport` field is a free string, unvalidated) | No structured remote form, no transport enum |
| Variable interpolation (`${env:VAR}`, `${workspaceFolder}`, etc.) | N/A (manifest-based, not textual interpolation) | Yes — resolves in `command`/`args`/`env`/`url`/`headers` | **None** — secrets must be typed literally into the JSON blob | **Critical: plaintext secrets in DB** |
| OAuth — dynamic client registration | Yes, directory connectors auto-handle it | Yes (DCR, falling back to static creds) | **None** — no OAuth flow wired to the MCP primitive at all (AgentMux *has* a general OAuth flow for Accounts, `oauth-catalog.ts` + `oauth_client.rs`, but it is not reachable from MCP server setup) | Missing entirely |
| OAuth — static client id/secret fallback | Advanced settings on custom connector | `auth` object in `mcp.json` (`CLIENT_ID`/`CLIENT_SECRET`/scopes) | None | Missing entirely |
| One-click install / catalog | Connectors directory (439+ verified integrations, 2-click OAuth) + `.dxt`/`.mcpb` double-click install | "Add Custom MCP" editor + community list links | Manual JSON only; Armory catalog only stores what a human already typed | Missing entirely |
| Health / liveness check before use | Implicit — connector shows connected/error state | Server list shows "Needs login" / error states | **None** — `mcp.upsert` only validates JSON syntax; a broken/unreachable server is indistinguishable from a working one until an agent tries to call it | **Critical UX gap — this is what the Ableton pilot targets** |
| Tool/resource/prompt introspection | Yes — tool list shown per connector | Yes — "MCP Tools" panel lists discovered tools | None — AgentMux never queries the server's `tools/list`; the config is trusted blind | Missing entirely |
| Per-tool / per-call approval | Permission prompt per tool call, scoped review at OAuth grant time | Prompt per tool call by default; "Yolo mode" to bypass; CLI `--approve-mcps` | None at the MCP layer (general tool-call UI exists — `ToolOverlayLog.tsx` — but has no approval gate concept) | Missing; intersects the deferred **Policy** primitive (`EXPLAINER_COMPOSABLE_MODEL…md` §7 item 2) |
| Trust pinned to resolved command, not just name (anti "MCPoison"/CVE-2025-54136) | N/A (sandboxed manifest execution) | Patched post-CVE — re-prompts on config change | **Partially accidental**: `is_global` servers are immutable to non-owners (`mcp.upsert` rejects mutating a global server) and per-agent private servers require an existing bind ref to edit — but there is no explicit "this config changed since you approved it" signal | Needs explicit config-hash pinning |
| Secrets never stored in plaintext | Enforced by connector/manifest model | Guidance-level (`${env:VAR}`) — still opt-in | Config blob stores literal values including secrets | Needs env-ref indirection into AgentMux's existing Account credential store |

---

## 4. Design

The guiding principle: **extend, don't replace.** `McpServer{id, name, transport, config, is_global}` stays the wire contract that `build_mcp_config_from_refs` merges into `.mcp.json` — every change below either adds optional structure *around* `config` or adds new fields that are additive at the DB/App-API layer, matching how MCP Servers + Skills were bolted on as primitives in Phase 1 without disturbing the older preset blob path.

### 4.1 Structured config + variable interpolation

Add a `kind: "stdio" | "http"` (replaces the free-text `transport` string, with `"stdio"` as the migrated default) and let the UI render one of two structured forms instead of a bare textarea:

- **stdio:** `command`, `args[]`, `env{}` (key/value rows, not one JSON blob)
- **http:** `url`, `headers{}`

Both forms still serialize to exactly the JSON object `agent_config.rs` already merges — no backend change to the merge path. The textarea remains available as an "Advanced / raw JSON" escape hatch (parity with Cursor, which lets you hand-edit `mcp.json` directly even though it also has structured UI).

Interpolation: support `${env:NAME}` inside `command`/`args`/`env`/`url`/`headers` values, resolved **at agent-launch config-build time** in `agent_config.rs` (not at rest) — so a value like `{"env": {"API_KEY": "${env:ABLETON_API_KEY}"}}` never persists a secret to SQLite. `${agentHome}`/`${workspaceFolder}` resolve the same way Cursor's do, using values `agent_config.rs` already has in scope for this agent.

### 4.2 Secrets via the existing Account primitive, not new plaintext storage

Rather than inventing a parallel secrets store, let an MCP server's `env`/`headers` values reference an **Account** (already a first-class primitive with its own encrypted-at-rest credential handling per `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md`) by id: `${account:<id>:token}`. This reuses infrastructure instead of building a second vault, and matches the composable-model philosophy ("reference, don't copy" — `EXPLAINER_COMPOSABLE_MODEL…md` §2).

### 4.3 OAuth for remote (`http`) MCP servers

Generalize the Accounts OAuth flow (`identity/oauth_client.rs` `config_for`, `oauth-catalog.ts`) from "known providers" (GitHub/Google/Slack) to **arbitrary remote MCP servers**, mirroring both vendors' two paths:

- **Dynamic Client Registration** (Cursor's default, matches MCP spec's DCR flow) — attempt first.
- **Static client id/secret fallback** (Cursor's `auth` object; Claude Desktop's "Advanced settings" on a custom connector) — when DCR isn't supported, the Armory MCP-server form collects `client_id`/`client_secret`/scopes and stores them as an Account, same as today's BYO OAuth path.

Redirect URI: reuse the existing AgentMux desktop-app callback scheme (already wired for Account OAuth) rather than inventing a second one, and identify the in-flight server the same way Cursor does — via the OAuth `state` param — since AgentMux, like Cursor, is a desktop app with one native callback handler.

### 4.4 Health / prerequisite probe

New App API command `mcp.probe(agent_id, id)`: opens a short-lived MCP client connection, performs the `initialize` handshake, and — on success — `tools/list` (and `resources/list`/`prompts/list` where the server declares those capabilities). Returns `{status: "connected"|"unreachable"|"handshake_failed", tool_count, resource_count, prompt_count, server_info, error}`.

Frontend: a new `mcp-capabilities.ts` module, structurally identical to `toolchain-capabilities.ts` (module-level `createStore`, `ensureCapability`/`watchCapability`, in-flight de-dup) — same pattern, new domain. The Armory MCP Servers list gets a status pill per server (Connected · N tools / Not connected / Error) instead of the current name-only row, polling while the Armory tab is open the same way the Toolchain widget polls Docker liveness.

This is the piece the Ableton pilot is written to prove out end-to-end (§6).

### 4.5 Tool/resource/prompt introspection + approval gate

`mcp.probe`'s tool list feeds two things:

1. **Read-only display** in the Armory (what does this server actually offer) and in the per-agent MCP tab (what will this agent be able to call) — parity with Cursor's "MCP Tools" panel and Claude's per-connector tool list.
2. **Per-tool enable/disable**, stored as a new `disabled_tools: string[]` field on the agent↔server bind row (`db_agent_mcp_ref`) — an agent can bind a server but suppress specific tools, same granularity Claude Desktop offers per connector.

Full per-call **approval prompting** (Cursor's "click Run tool to proceed" / Claude's permission dialog) is **out of scope for this spec's Phase A–C** — it is correctly the deferred **Policy** primitive's job (`EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md` §7 item 2: "hooks + `.claude/settings.json` permissions... deferred to a later phase"). This spec only prepares the ground: `disabled_tools` and the introspected tool list are exactly the inputs a future Policy primitive needs, so they should land now even though enforcement lands later.

### 4.6 Catalog / one-click install

A small built-in manifest catalog (Armory → MCP Servers → "+ Browse catalog", alongside the existing "+ New MCP server"), each entry describing: display name, `kind`, templated `config` (with `${...}` placeholders for anything user-specific), required env vars with human labels, a free-text **prerequisites** field, and a docs link. Selecting an entry pre-fills the structured form (§4.1) instead of requiring the user to author JSON from scratch — this is AgentMux's analogue of Claude's Connectors directory and Smithery-style `npx @smithery/cli install`, scoped down to "pre-filled form," not a remote package registry (no new supply-chain trust surface introduced by this spec).

### 4.7 Trust hardening (config-hash pinning)

Record a hash of the resolved `(command, args, config)` tuple at bind time. If a **global** server's config changes after an agent has bound it, `mcp.list` flags it `needs_reapproval: true` instead of silently picking up the new command on next launch — directly closing the CVE-2025-54136 ("MCPoison") pattern where trust was bound to a server *name* rather than what it actually executes. New entries appearing in a global catalog already require an explicit bind (§ current state) — this closes the "already-bound server gets silently redefined" gap CVE-2025-54135 also touched on.

---

## 5. Data model changes

Additive only — no destructive migration:

```sql
-- db_mcp_servers: transport stays TEXT but the value set narrows to "stdio"|"http";
-- existing free-text values that aren't one of those two stay valid (raw-JSON escape hatch).
ALTER TABLE db_mcp_servers ADD COLUMN config_hash   TEXT NOT NULL DEFAULT '';
ALTER TABLE db_mcp_servers ADD COLUMN prereq_note   TEXT NOT NULL DEFAULT ''; -- catalog-sourced remediation text (§4.4/§6)

-- db_agent_mcp_ref: per-bind state, not per-server
ALTER TABLE db_agent_mcp_ref ADD COLUMN disabled_tools TEXT NOT NULL DEFAULT '[]'; -- JSON string[]
ALTER TABLE db_agent_mcp_ref ADD COLUMN approved_hash  TEXT NOT NULL DEFAULT ''; -- config_hash at bind time (§4.7)
```

No change to `build_mcp_config_from_refs`'s output shape — `config_hash`/`prereq_note`/`disabled_tools`/`approved_hash` are all Armory/App-API-side metadata, never written into `.mcp.json`.

## 6. Real-world pilot: Ableton MCP

**Why this one first.** Ableton MCP is `stdio` transport (no OAuth needed — keeps the pilot decoupled from §4.3, the highest-risk piece), needs no secrets (keeps it decoupled from §4.2), and its entire failure mode today is exactly §4.4's gap: nothing in AgentMux can currently tell a user *why* the agent's tool calls are hanging or erroring when the actual cause is "you haven't opened Ableton Live yet" or "the Remote Script isn't installed in the right folder for your Live version."

**What ahujasid/ableton-mcp requires** (verified against upstream docs, 2026-07-08):
- Ableton Live 10+, Python 3.8+, `uv`/`uvx` on PATH.
- A Remote Script (`AbletonMCP_Remote_Script/__init__.py`) manually copied into Ableton's Remote Scripts folder — **Live 10.1.13 through 12 requires the User Library path** (`%USERPROFILE%\Documents\Ableton\User Library\Remote Scripts\AbletonMCP\`), not the older Preferences path, which Live 12+ no longer scans.
- Ableton's Preferences → Link/MIDI → Control Surface set to `AbletonMCP` (Input/Output: None — it's a socket server, not a MIDI device).
- A TCP socket on `localhost:9877` (default; user-changeable, but both the Remote Script and the MCP server's config must agree).
- The MCP server itself: `{"command": "uvx", "args": ["ableton-mcp"]}` — no persisted secret.

**Catalog entry (§4.6):**

```json
{
  "name": "Ableton Live",
  "kind": "stdio",
  "config": { "command": "uvx", "args": ["ableton-mcp"] },
  "prereq_note": "Requires Ableton Live 10+ running with the AbletonMCP Remote Script installed and selected as the active Control Surface (Preferences → Link, Tempo & MIDI). See docs link.",
  "docs_url": "https://github.com/ahujasid/ableton-mcp"
}
```

**Health probe (§4.4) applied here:** `mcp.probe` attempts the stdio handshake; `uvx ableton-mcp` starts regardless of Ableton's state (it's the Remote Script's socket connect that fails), so the probe's `handshake_failed` / timeout case is what actually surfaces — the Armory status pill should read **"MCP process started, but Ableton isn't responding on 9877 — is Live running with AbletonMCP selected as Control Surface?"** rather than a bare "Error." This is the concrete acceptance bar for §4.4, not a generic "connected/not connected" toggle: the pilot is done when a user who hasn't opened Ableton yet sees *that* sentence in the Armory, in-app, before ever spawning an agent — not a stack trace three tool calls deep in a transcript.

**Non-goals for the pilot:** no OAuth (not needed), no tool-approval enforcement (§4.5 stays introspection-only for this pilot), no bundling/packaging the Remote Script inside AgentMux — installing it into Ableton's folder remains a documented manual step (linked from `prereq_note`), same as it is for every other MCP client; AgentMux does not attempt to write into a third-party app's data directory on the user's behalf.

## 7. Phasing

| Phase | Scope | Depends on |
|---|---|---|
| **A** | §4.1 structured config UI + interpolation, §4.4 health probe + `mcp-capabilities.ts`, §5 migration | none — pure additive extension of the shipped Phase 1 primitive |
| **B** | §4.6 catalog UI + the Ableton catalog entry + pilot acceptance bar (§6) | Phase A (needs the probe to render the remediation string) |
| **C** | §4.2 secrets-via-Account indirection, §4.3 OAuth (DCR + static fallback) | Phase A; reuses existing Account/OAuth infra, additive |
| **D** | §4.5 tool introspection/disable, §4.7 config-hash pinning | Phase A; D's approval-gate *enforcement* is explicitly deferred to the future Policy primitive — D here only stores the data it needs |

Ableton MCP (Phase B) intentionally ships **before** OAuth (Phase C) — it validates the highest-value, lowest-risk slice (structured config + health probe) end-to-end on a real third-party server before the OAuth surface (the piece with the largest security blast radius per §3's CVE notes) gets built.

## 8. Open questions for product decision

1. Does the catalog (§4.6) ship with just Ableton MCP as a proof of concept, or a small starter set (filesystem, GitHub, Slack — mirroring what's already got Account-provider wiring in `oauth-catalog.ts`)? Recommend: ship with Ableton MCP alone for Phase B; expand the catalog only after the pattern is validated.
2. Where does `disabled_tools` enforcement actually live once the Policy primitive exists — does Policy subsume `db_agent_mcp_ref.disabled_tools`, or does Policy read it as an input? Needs a decision before Phase D locks its schema.
3. Should `mcp.probe` run automatically (e.g., on every Armory MCP tab mount, like Docker's liveness probe) or only on explicit "Test connection" click? Automatic matches Claude/Cursor's ambient status; but an MCP server's `initialize` handshake may have side effects (some servers log a connection, spin up resources) that a passive diagnostics view shouldn't trigger repeatedly. Recommend: on-mount + manual refresh + the same 4s poll-while-open pattern `toolchain-capabilities.ts` already uses for Docker, since that precedent already made this call for a comparable case.

## 9. Sources

- [Claude Help Center — Getting started with local MCP servers](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop)
- [Claude Help Center — Custom connectors via remote MCP](https://support.claude.com/en/articles/11503834-build-custom-connectors-via-remote-mcp-servers)
- [Claude Platform Docs — MCP connector](https://platform.claude.com/docs/en/agents-and-tools/mcp-connector)
- [Pluto Security — Claude extension ecosystem security map](https://pluto.security/blog/claude-extension-ecosystem-security-practitioner-guide/) (DXT sandboxing / injection-chaining incident)
- [Cursor Docs — MCP](https://cursor.com/docs/mcp)
- [TrueFoundry — MCP Authentication in Cursor](https://www.truefoundry.com/blog/mcp-authentication-in-cursor-oauth-api-keys-and-secure-configuration) (DCR + static OAuth, CVE-2025-54136/54135)
- [WorkOS — Understanding MCP features: Tools, Resources, Prompts, Sampling, Roots, Elicitation](https://workos.com/blog/mcp-features-guide)
- [ahujasid/ableton-mcp](https://github.com/ahujasid/ableton-mcp)
- [mcp.directory — Ableton Live MCP Server install & setup](https://mcp.directory/servers/ableton-live)
