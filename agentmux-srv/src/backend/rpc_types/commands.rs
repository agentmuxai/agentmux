// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RPC command-name string constants (match Go's `wshrpc.Command_*` constants)
//! plus assorted size/type/client constants.

// ---- Size/type constants (match Go) ----

pub const MAX_FILE_SIZE: usize = 50 * 1024 * 1024; // 50M
pub const MAX_DIR_SIZE: usize = 1024;
pub const FILE_CHUNK_SIZE: usize = 64 * 1024;
pub const DIR_CHUNK_SIZE: usize = 128;

pub const LOCAL_CONN_NAME: &str = "local";

// ---- RPC type constants ----

pub const RPC_TYPE_CALL: &str = "call";
pub const RPC_TYPE_RESPONSE_STREAM: &str = "responsestream";
pub const RPC_TYPE_STREAMING_REQUEST: &str = "streamingrequest";
pub const RPC_TYPE_COMPLEX: &str = "complex";

// ---- CreateBlock action constants ----

pub const CREATE_BLOCK_ACTION_REPLACE: &str = "replace";
pub const CREATE_BLOCK_ACTION_SPLIT_UP: &str = "splitup";
pub const CREATE_BLOCK_ACTION_SPLIT_DOWN: &str = "splitdown";
pub const CREATE_BLOCK_ACTION_SPLIT_LEFT: &str = "splitleft";
pub const CREATE_BLOCK_ACTION_SPLIT_RIGHT: &str = "splitright";

// ---- Command constants (match Go's wshrpc.Command_* constants) ----

// Special commands
pub const COMMAND_ROUTE_ANNOUNCE: &str = "routeannounce";
pub const COMMAND_ROUTE_UNANNOUNCE: &str = "routeunannounce";

// Core commands
pub const COMMAND_GET_META: &str = "getmeta";
pub const COMMAND_SET_META: &str = "setmeta";

// Controller commands
pub const COMMAND_CONTROLLER_INPUT: &str = "controllerinput";
pub const COMMAND_CONTROLLER_RESYNC: &str = "controllerresync";

/// Create a headless sub-block (no tab/layout entry) parented to an
/// existing block — e.g. a `term`-view PTY embedded in an agent pane's
/// details drawer. Spec: docs/specs/SPEC_AGENT_SHELL_XTERM_TERMINAL_2026_07_03.md §4.
pub const COMMAND_CREATE_SUB_BLOCK: &str = "createsubblock";
/// Tear down a sub-block created via `createsubblock`: kills its
/// controller first, then deletes the block row and unlinks it from
/// its parent's `subblockids`.
pub const COMMAND_DELETE_SUB_BLOCK: &str = "deletesubblock";

/// Per-tool-call permission decision RPC. Frontend sends after the
/// user clicks Allow / Deny in `AgentDecisionPanel`. Today the
/// handler validates the payload and logs the decision (audit
/// trail); actual delivery to the agent CLI — rules persistence
/// vs. interactive subprocess — is deferred to PR-3b/PR-4 per
/// docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §9.1.
pub const COMMAND_TOOL_DECISION: &str = "tooldecision";

// Subprocess agent commands
pub const COMMAND_SUBPROCESS_SPAWN: &str = "subprocessspawn";
pub const COMMAND_AGENT_INPUT: &str = "agentinput";
/// Deliver an AskUserQuestion answer to the running agent CLI as a tool_result.
/// Lowercase, no separators — matches the sibling command-name convention
/// (`agentinput`, `agentstop`, `tooldecision`).
/// Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md.
pub const COMMAND_AGENT_ANSWER: &str = "agentanswer";
pub const COMMAND_AGENT_STOP: &str = "agentstop";
pub const COMMAND_SHELL_EXEC: &str = "shellexec";
/// Stop a running persistent shell node (Phase 3) — UI stop button.
pub const COMMAND_SHELL_STOP: &str = "shellstop";
pub const COMMAND_WRITE_AGENT_CONFIG: &str = "writeagentconfig";
pub const COMMAND_RESOLVE_CLI: &str = "resolvecli";
pub const COMMAND_CHECK_CLI_AUTH: &str = "checkcliauth";

// Block commands

// File commands

// Event commands
pub const COMMAND_EVENT_RECV: &str = "eventrecv";
pub const COMMAND_EVENT_SUB: &str = "eventsub";
pub const COMMAND_EVENT_UNSUB: &str = "eventunsub";
pub const COMMAND_EVENT_UNSUB_ALL: &str = "eventunsuball";
pub const COMMAND_EVENT_READ_HISTORY: &str = "eventreadhistory";

// Stream/test commands

// Config commands
pub const COMMAND_SET_CONFIG: &str = "setconfig";
pub const COMMAND_GET_FULL_CONFIG: &str = "getfullconfig";

// Remote commands

// Info/activity commands
pub const COMMAND_APP_INFO: &str = "waveinfo";

// Connection commands
// COMMAND_CONN_REINSTALL_WSH / COMMAND_CONN_UPDATE_WSH / COMMAND_DISMISS_WSH_FAIL
// have been removed — wsh has been retired. See
// specs/SPEC_RETIRE_WSH_2026_04_12.md.

// Workspace commands

// UI commands

// VDom commands

// AI commands
pub const COMMAND_GET_AI_RATE_LIMIT: &str = "getwaveairatelimit";

// Screenshot

// RT info

// Terminal

// Agent
pub const COMMAND_LIST_AGENTS: &str = "listagents";
pub const COMMAND_CREATE_AGENT: &str = "createagent";
pub const COMMAND_UPDATE_AGENT: &str = "updateagent";
pub const COMMAND_DELETE_AGENT: &str = "deleteagent";
pub const COMMAND_GET_AGENT_CONTENT: &str = "getagentcontent";
pub const COMMAND_SET_AGENT_CONTENT: &str = "setagentcontent";
pub const COMMAND_GET_ALL_AGENT_CONTENT: &str = "getallagentcontent";

// Agent Skills
pub const COMMAND_LIST_AGENT_SKILLS: &str = "listagentskills";
pub const COMMAND_CREATE_AGENT_SKILL: &str = "createagentskill";
pub const COMMAND_UPDATE_AGENT_SKILL: &str = "updateagentskill";
pub const COMMAND_DELETE_AGENT_SKILL: &str = "deleteagentskill";

// Agent History
pub const COMMAND_APPEND_AGENT_HISTORY: &str = "appendagenthistory";
pub const COMMAND_LIST_AGENT_HISTORY: &str = "listagenthistory";
pub const COMMAND_SEARCH_AGENT_HISTORY: &str = "searchagenthistory";

// Agent Import
pub const COMMAND_IMPORT_AGENT_FROM_CLAW: &str = "importagentfromclaw";
pub const COMMAND_IMPORT_AGENTS: &str = "importagents";

// Agent Export
pub const COMMAND_EXPORT_AGENTS: &str = "exportagents";

// Agent Seed
pub const COMMAND_RESEED_AGENTS: &str = "reseedagents";

// Identity accounts (v6 — replaces localStorage)
pub const COMMAND_LIST_IDENTITY_ACCOUNTS: &str = "listidentityaccounts";
pub const COMMAND_GET_IDENTITY_ACCOUNT: &str = "getidentityaccount";
pub const COMMAND_UPSERT_IDENTITY_ACCOUNT: &str = "upsertidentityaccount";
pub const COMMAND_DELETE_IDENTITY_ACCOUNT: &str = "deleteidentityaccount";
/// Armory: validate (optional, user-initiated) + securely store an API
/// key. The plaintext goes to the OS keychain; the DB keeps only a
/// `SecretRef::Keychain` pointer + masked tail + metadata. Used for both new
/// accounts and replacing a key on an existing one (via `accountId`).
/// See specs/SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
pub const COMMAND_ACCOUNT_KEY_VERIFY: &str = "account.key.verify";
/// Armory service OAuth (scaffold — activates once client ids are
/// provisioned or supplied as BYO). See SPEC_TRUST_CENTER_2026_06_15.md §4.2.
pub const COMMAND_ACCOUNT_OAUTH_START: &str = "account.oauth.start";
pub const COMMAND_ACCOUNT_OAUTH_POLL: &str = "account.oauth.poll";
pub const COMMAND_ACCOUNT_OAUTH_CANCEL: &str = "account.oauth.cancel";

// Agent ↔ Identity junction
pub const COMMAND_LINK_AGENT_IDENTITY: &str = "linkagentidentity";
pub const COMMAND_UNLINK_AGENT_IDENTITY: &str = "unlinkagentidentity";
pub const COMMAND_LIST_AGENT_IDENTITIES: &str = "listagentidentities";

// Identity bundles (v7 — named credential bundles)
pub const COMMAND_LIST_IDENTITY_BUNDLES: &str = "listidentitybundles";
pub const COMMAND_GET_IDENTITY_BUNDLE: &str = "getidentitybundle";
pub const COMMAND_UPSERT_IDENTITY_BUNDLE: &str = "upsertidentitybundle";
pub const COMMAND_DELETE_IDENTITY_BUNDLE: &str = "deleteidentitybundle";
pub const COMMAND_BIND_IDENTITY_ACCOUNT: &str = "bindidentityaccount";
pub const COMMAND_UNBIND_IDENTITY_ACCOUNT: &str = "unbindidentityaccount";
pub const COMMAND_LIST_IDENTITY_BINDINGS: &str = "listidentitybindings";

// Memory bundles (v7 — agent personality / capability stack)
pub const COMMAND_LIST_MEMORIES: &str = "listmemories";
pub const COMMAND_GET_MEMORY: &str = "getmemory";
pub const COMMAND_UPSERT_MEMORY: &str = "upsertmemory";
pub const COMMAND_DELETE_MEMORY: &str = "deletememory";
/// v9 — set the global-brain section order. `ids` is the full ordered list
/// of global bundle ids; each row's `sort_order` becomes its index.
pub const COMMAND_REORDER_GLOBAL_BRAIN: &str = "reorderglobalbrain";

// Agent instances
pub const COMMAND_LIST_AGENT_INSTANCES: &str = "listagentinstances";
pub const COMMAND_GET_AGENT_INSTANCE: &str = "getagentinstance";
pub const COMMAND_CREATE_AGENT_INSTANCE: &str = "createagentinstance";
pub const COMMAND_UPDATE_AGENT_INSTANCE: &str = "updateagentinstance";
pub const COMMAND_DELETE_AGENT_INSTANCE: &str = "deleteagentinstance";
/// v8 — list named agent instances for the launch modal's "Continue
/// agent" dropdown. Filters to non-hidden rows with a non-empty
/// instance_name, joined with definition + identity + memory bundles.
pub const COMMAND_LIST_NAMED_AGENTS: &str = "listnamedagents";
/// v8 — soft-delete (hide) a named agent instance from the dropdown.
/// Row + working directory remain on disk for audit + recovery.
pub const COMMAND_HIDE_NAMED_AGENT: &str = "hidenamedagent";
/// Cascade follow-up (2026-05-23) — list recent agent sessions with
/// conversation previews extracted from the filestore `output.state.json`
/// snapshot. Powers the AgentPicker's "Recent sessions" surface so a
/// pane crash that orphans a conversation becomes recoverable from
/// normal UI. See `docs/recovery/MAKS_CONVERSATION_2026_05_23.md`.
pub const COMMAND_LIST_RECENT_SESSIONS: &str = "listrecentsessions";

// Agent definition branching
pub const COMMAND_FORK_AGENT_DEFINITION: &str = "forkagentdefinition";
/// Returns the suggested branch label for a fork without mutating anything.
/// Called when the user clicks "Open new session" to pre-fill the name input.
pub const COMMAND_FORK_AGENT_DEFINITION_SUGGEST: &str = "forkagentdefinitionsuggest";

/// Two-tier picker (Phase 1 — SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
/// Clone a seeded template into a new user-owned agent definition with
/// `is_seeded = 0`. Copies provider + cmd + env + auth-config fields
/// from the template, applies the caller-supplied name + bindings,
/// returns the new definition_id so the frontend can immediately
/// launch. Rejects non-template ids + duplicate user-agent names.
pub const COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE: &str = "agentdefcreatefromtemplate";

/// Returns whether a usable container runtime is reachable RIGHT NOW —
/// i.e. the Docker daemon answers a `ping`, not merely that the `docker`
/// CLI is on PATH. Used by the create-from-template modal to decide
/// whether to offer/default the container runtime; a binary-only check
/// would false-positive when Docker is installed but the daemon is
/// stopped, steering the user into a container agent that can't start.
/// Response: `{ "available": bool }`.
pub const COMMAND_CONTAINER_RUNTIME_AVAILABLE: &str = "containerruntimeavailable";

/// Two-tier picker (Phase 2 — SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md
/// Q2 Decision Y). Set the `user_hidden` flag on a seeded template so
/// it disappears from the default `+ New from template` list. Idempotent;
/// rejects user-owned (`is_seeded = 0`) definitions — those use
/// `deleteagent` instead. Manifest re-sync resets `user_hidden = 0` for
/// any newly-added template id so fresh templates always surface once.
pub const COMMAND_AGENT_DEF_HIDE: &str = "agentdefhide";
/// Two-tier picker (Phase 2). Inverse of `agentdefhide` — set
/// `user_hidden = 0` so a previously-hidden template reappears in the
/// picker's templates tier. Powers the settings "Hidden templates"
/// unhide affordance. Same validation as hide.
pub const COMMAND_AGENT_DEF_UNHIDE: &str = "agentdefunhide";
/// Two-tier picker (Phase 2). Return only the hidden templates
/// (`is_seeded = 1 AND user_hidden = 1`). Backs the settings UI's
/// list of templates the user can unhide. The picker proper never
/// calls this — it uses `listagents` (which excludes hidden rows
/// by default).
pub const COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES: &str = "agentdeflisthiddentemplates";

// Drone pane (v8 — issue #753 Phase 1)
pub const COMMAND_LIST_DRONES: &str = "listdrones";
pub const COMMAND_GET_DRONE: &str = "getdrone";
pub const COMMAND_UPSERT_DRONE: &str = "upsertdrone";
pub const COMMAND_DELETE_DRONE: &str = "deletedrone";
pub const COMMAND_RUN_DRONE: &str = "rundrone";
pub const COMMAND_LIST_DRONE_RUNS: &str = "listdroneruns";

// App API Tier 1 — agent lifecycle commands
pub const COMMAND_AGENT_OPEN: &str = "agent.open";
pub const COMMAND_AGENT_SEND: &str = "agent.send";
pub const COMMAND_AGENT_STOP_API: &str = "agent.stop";
pub const COMMAND_AGENT_STATUS: &str = "agent.status";
pub const COMMAND_AGENT_LIST: &str = "agent.list";
pub const COMMAND_AGENT_OUTPUT: &str = "agent.output";
/// List every OS process currently tracked for a given agent block.
/// Returns `AgentProcessListResult`. Consumed by the swarm activity
/// panel. See `backend::process_tracker`.
pub const COMMAND_AGENT_PROCESS_LIST: &str = "agent.process-list";
/// List every block currently tracked (for the swarm aggregate view).
/// Returns `AgentTrackedBlocksResult`.
pub const COMMAND_AGENT_TRACKED_BLOCKS: &str = "agent.tracked-blocks";
/// Terminate a single process by PID if it's a member of a given
/// block's tracker tree. Silently no-ops if the PID isn't tracked.
/// Returns `AgentKillResult { ok: bool }`.
pub const COMMAND_AGENT_KILL_PROCESS: &str = "agent.kill-process";
/// Terminate the entire process tree for a given block.
/// On Windows: `TerminateJobObject`. On Linux: `cgroup.kill`. On
/// macOS: `killpg`. Returns `AgentKillResult { ok: true }` even when
/// there are no members (idempotent).
pub const COMMAND_AGENT_KILL_TREE: &str = "agent.kill-tree";
/// Create or upsert an agent definition. Broadcasts `agents:changed` on
/// success so all open frontends refresh My Agents without a restart.
pub const COMMAND_AGENT_DEFINE: &str = "agent.define";

// App API Tier 2 — pane lifecycle commands
pub const COMMAND_PANE_OPEN: &str = "pane.open";

// App API Tier 1 — blockfile pagination commands
pub const COMMAND_BLOCKFILE_LINE_COUNT: &str = "blockfile:line_count";
pub const COMMAND_BLOCKFILE_READ_RANGE: &str = "blockfile:read_range";
pub const COMMAND_BLOCKFILE_READ_STATE: &str = "blockfile:read_state";
pub const COMMAND_BLOCKFILE_WRITE_STATE: &str = "blockfile:write_state";

// App API — identity/preset/memory namespaces
pub const COMMAND_IDENTITY_SELF_ACCOUNTS: &str = "identity.self.accounts";
pub const COMMAND_IDENTITY_ACCOUNT_UPSERT: &str = "identity.account.upsert";
pub const COMMAND_IDENTITY_ACCOUNT_VALIDATE: &str = "identity.account.validate";
pub const COMMAND_IDENTITY_SELF_UNLINK: &str = "identity.self.unlink";
// Bundle App API commands (Preset → Bundle, spec Phase 2). The `preset.*`
// constants below are retained as wire aliases for one release.
pub const COMMAND_BUNDLE_LIST: &str = "bundle.list";
pub const COMMAND_BUNDLE_GET: &str = "bundle.get";
pub const COMMAND_BUNDLE_UPSERT: &str = "bundle.upsert";
pub const COMMAND_BUNDLE_DELETE: &str = "bundle.delete";
pub const COMMAND_BUNDLE_SELF_GET: &str = "bundle.self.get";
// Deprecated `preset.*` aliases — kept wired for one release (remove in Phase 4).
pub const COMMAND_PRESET_LIST: &str = "preset.list";
pub const COMMAND_PRESET_GET: &str = "preset.get";
pub const COMMAND_PRESET_UPSERT: &str = "preset.upsert";
pub const COMMAND_PRESET_DELETE: &str = "preset.delete";
pub const COMMAND_PRESET_SELF_GET: &str = "preset.self.get";
pub const COMMAND_MEMORY_LIST: &str = "memory.list";
pub const COMMAND_MEMORY_READ: &str = "memory.read";
pub const COMMAND_MEMORY_WRITE: &str = "memory.write";

// App API — v1 standalone Skill primitives
pub const COMMAND_SKILL_LIST: &str = "skill.list";
pub const COMMAND_SKILL_GET: &str = "skill.get";
pub const COMMAND_SKILL_UPSERT: &str = "skill.upsert";
pub const COMMAND_SKILL_DELETE: &str = "skill.delete";
pub const COMMAND_SKILL_BIND: &str = "skill.bind";
pub const COMMAND_SKILL_UNBIND: &str = "skill.unbind";

// App API — v1 standalone MCP Server primitives
pub const COMMAND_MCP_LIST: &str = "mcp.list";
pub const COMMAND_MCP_GET: &str = "mcp.get";
pub const COMMAND_MCP_UPSERT: &str = "mcp.upsert";
pub const COMMAND_MCP_DELETE: &str = "mcp.delete";
pub const COMMAND_MCP_BIND: &str = "mcp.bind";
pub const COMMAND_MCP_UNBIND: &str = "mcp.unbind";

// App API Tier 1 — session archival commands
pub const COMMAND_SESSION_ARCHIVE: &str = "session:archive";
pub const COMMAND_SESSION_RESTORE: &str = "session:restore";
pub const COMMAND_SESSION_EXPORT: &str = "session:export";

// Per-turn live activity summary (Haiku-powered, writes term:activity)
pub const COMMAND_SESSION_ACTIVITY_SUMMARY: &str = "session:activity_summary";

// Option E (PR 1 of 2) — agent-anchored session zones.
// A session zone is bound to the *agent definition* (`definition_id`),
// not the identity bundle. Every block of the same agent reads/writes
// through `agent:<defId>:current`; archiving snapshots to
// `agent:<defId>:archive:<ts_ms>`. See
// docs/specs/SPEC_CONTINUATION_SESSION_PERSISTENCE_2026_05_23.md.
pub const COMMAND_AGENT_SESSION_READ: &str = "agent:session:read";
pub const COMMAND_AGENT_SESSION_WRITE_STATE: &str = "agent:session:write_state";
pub const COMMAND_AGENT_SESSION_APPEND_OUTPUT: &str = "agent:session:append_output";
pub const COMMAND_AGENT_SESSION_ARCHIVE: &str = "agent:session:archive";
pub const COMMAND_AGENT_SESSION_LIST_ARCHIVES: &str = "agent:session:list_archives";

// ---- Native memory RPCs (Phase 2 — agent:memory:list / read / write) ----
pub const COMMAND_NATIVE_MEMORY_LIST: &str = "agent:memory:list";
pub const COMMAND_NATIVE_MEMORY_READ_FILE: &str = "agent:memory:read_file";
pub const COMMAND_NATIVE_MEMORY_WRITE_FILE: &str = "agent:memory:write_file";

// ---- Client type constants ----

pub const CLIENT_TYPE_CONN_SERVER: &str = "connserver";
pub const CLIENT_TYPE_BLOCK_CONTROLLER: &str = "blockcontroller";

// ---- Tool store commands ----

pub const COMMAND_GET_TOOL_STATUS: &str = "gettoolstatus";
pub const COMMAND_INSTALL_TOOL: &str = "installtool";
