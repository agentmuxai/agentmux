// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hand-maintained type bindings. Keep in sync with agentmux-srv (backend/obj.rs,
// rpc_types.rs) and the wshrpc wire types. The original Go generator
// (cmd/generate/main-generatets.go) was removed with the Go backend.

declare global {

    // wshrpc.AIAttachedFile
    type AIAttachedFile = {
        name: string;
        type: string;
        size: number;
        data64: string;
    };

    // wshrpc.ActivityDisplayType
    type ActivityDisplayType = {
        width: number;
        height: number;
        dpr: number;
        internal?: boolean;
    };

    // wshrpc.ActivityUpdate
    type ActivityUpdate = {
        fgminutes?: number;
        activeminutes?: number;
        openminutes?: number;
        waveaifgminutes?: number;
        waveaiactiveminutes?: number;
        numtabs?: number;
        newtab?: number;
        numblocks?: number;
        numwindows?: number;
        numws?: number;
        numwsnamed?: number;
        numsshconn?: number;
        numwslconn?: number;
        nummagnify?: number;
        numpanics?: number;
        numaireqs?: number;
        startup?: number;
        shutdown?: number;
        settabtheme?: number;
        buildtime?: string;
        displays?: ActivityDisplayType[];
        renderers?: {[key: string]: number};
        blocks?: {[key: string]: number};
        conn?: {[key: string]: number};
    };

    // wshrpc.AiMessageData
    type AiMessageData = {
        message?: string;
    };

    // waveobj.Block
    type Block = WaveObj & {
        parentoref?: string;
        runtimeopts?: RuntimeOpts;
        stickers?: StickerType[];
        subblockids?: string[];
    };

    // blockcontroller.BlockControllerRuntimeStatus
    type BlockControllerRuntimeStatus = {
        blockid: string;
        version: number;
        shellprocstatus?: string;
        shellprocconnname?: string;
        shellprocexitcode: number;
        spawn_ts_ms?: number;
        is_agent_pane?: boolean;
        // True if a turn is in flight (message sent, no terminating "result"
        // event observed yet). Only meaningful for persistent/ACP agent
        // controllers with a health monitor wired to the NDJSON stream —
        // absent/false for shell/PTY-backed panes, which have no such signal.
        turn_active?: boolean;
    };

    // agents.failure.AgentFailure — payload of the `agentfailure` wave event
    // (classified cause of a non-zero agent exit). snake_case `code`, camelCase rest.
    type AgentFailure = {
        code:
            | "rate_limited"
            | "overloaded"
            | "usage_limit"
            | "auth"
            | "context_exceeded"
            | "max_turns"
            | "network"
            | "killed"
            | "no_output"
            | "spawn_failure"
            | "unknown_non_zero"
            | "unresponsive";
        title: string;
        detail: string;
        exitCode?: number;
        signal?: number;
        stderrTail?: string;
        retryable: boolean;
    };

    // waveobj.BlockDef
    type BlockDef = {
        files?: {[key: string]: FileDef};
        meta?: MetaType;
    };

    // wshrpc.BlockInfoData
    type BlockInfoData = {
        blockid: string;
        tabid: string;
        workspaceid: string;
        block: Block;
        files: FileInfo[];
    };

    // webcmd.BlockInputWSCommand
    type BlockInputWSCommand = {
        wscommand: "blockinput";
        blockid: string;
        inputdata64: string;
    };

    // wshrpc.BlocksListEntry
    type BlocksListEntry = {
        windowid: string;
        workspaceid: string;
        tabid: string;
        blockid: string;
        meta: MetaType;
    };

    // wshrpc.BlocksListRequest
    type BlocksListRequest = {
        windowid?: string;
        workspaceid?: string;
    };

    // waveobj.Client
    type Client = WaveObj & {
        windowids: string[];
        tosagreed?: number;
        hasoldhistory?: boolean;
        tempoid?: string;
    };

    // workspaceservice.CloseTabRtnType
    type CloseTabRtnType = {
        closewindow?: boolean;
        newactivetabid?: string;
    };

    // wshrpc.CommandAppendIJsonData
    type CommandAppendIJsonData = {
        zoneid: string;
        filename: string;
        data: {[key: string]: any};
    };

    // wshrpc.CommandAuthenticateRtnData
    type CommandAuthenticateRtnData = {
        routeid: string;
        authtoken?: string;
        env?: {[key: string]: string};
        initscripttext?: string;
    };

    // wshrpc.CommandAuthenticateTokenData
    type CommandAuthenticateTokenData = {
        token: string;
    };

    // wshrpc.CommandBlockInputData
    type CommandBlockInputData = {
        blockid: string;
        inputdata64?: string;
        signame?: string;
        termsize?: TermSize;
    };

    // wshrpc.CommandToolDecisionData — per-tool-call permission reply.
    // Spec: docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §4.3.
    type CommandToolDecisionData = {
        blockid: string;
        request_id: string;
        outcome: "allow" | "deny";
        scope: "once" | "session" | "project" | "global";
        feedback?: string;
    };

    // wshrpc.CommandDockNodeStatusData — fire-and-forget push whenever a
    // ToolNode's status changes. Backs `muxspect dock`. Spec:
    // docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.1.
    type CommandDockNodeStatusData = {
        blockid: string;
        node_id: string;
        tool_name: string;
        status: string;
        timestamp?: number;
        run_in_background?: boolean;
    };

    // CommandAgentAnswerData — AskUserQuestion answer, delivered to the running
    // agent CLI via the Agent SDK control protocol (a control_response carrying
    // updatedInput.answers). Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    type CommandAgentAnswerData = {
        blockid: string;
        tool_use_id: string;
        // question text → chosen label | label[] (multiSelect) | free-text ("Other")
        answers: {[key: string]: string | string[]};
    };

    // wshrpc.CommandBlockSetViewData
    type CommandBlockSetViewData = {
        blockid: string;
        view: string;
    };

    // wshrpc.CommandCaptureBlockScreenshotData
    type CommandCaptureBlockScreenshotData = {
        blockid: string;
    };

    // wshrpc.CommandControllerAppendOutputData
    type CommandControllerAppendOutputData = {
        blockid: string;
        data64: string;
    };

    // wshrpc.CommandControllerResyncData
    type CommandControllerResyncData = {
        forcerestart?: boolean;
        tabid: string;
        blockid: string;
        rtopts?: RuntimeOpts;
    };

    // wshrpc.CommandCreateBlockData
    type CommandCreateBlockData = {
        tabid: string;
        blockdef: BlockDef;
        rtopts?: RuntimeOpts;
        magnified?: boolean;
        ephemeral?: boolean;
        focused?: boolean;
        targetblockid?: string;
        targetaction?: string;
    };

    // wshrpc.CommandCreateSubBlockData
    type CommandCreateSubBlockData = {
        parentblockid: string;
        blockdef: BlockDef;
    };

    // AgentDefinition
    type AgentDefinition = {
        id: string;
        // Stable, filesystem-safe identifier. Drives working dir, env vars,
        // and cross-references. NEVER changes — distinct from `name` which
        // is the renameable display. See
        // specs/SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md.
        slug: string;
        name: string;
        icon: string;
        provider: string;
        description: string;
        working_directory: string;
        shell: string;
        provider_flags: string;
        auto_start: number;
        restart_on_crash: number;
        idle_timeout_minutes: number;
        created_at: number;
        agent_type: string;
        environment: string;
        agent_bus_id: string;
        is_seeded: number;
        /**
         * JSON-encoded per-provider account refs.
         * **Deprecated in v6** — use `db_agent_identity_links` (junction
         * table) via `listAgentIdentities` RPC instead. Kept on the type
         * for compatibility with rows that still carry the legacy blob.
         */
        accounts?: string;
        /**
         * Forked-from definition id, or empty string for root definitions.
         * Added in v6. See specs/archive/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md.
         */
        parent_id?: string;
        /**
         * Free-form label describing the branch (e.g. "pr-422-review").
         * Empty for root definitions. Added in v6.
         */
        branch_label?: string;
        /**
         * Last-modified timestamp (epoch ms). Set to created_at on insert,
         * refreshed on every update. Schema v2. `0` for rows last written
         * before v2.
         */
        updated_at?: number;
        /**
         * Per-user hide flag for seeded templates (0 = visible, 1 = hidden).
         * Schema v3. Set via AgentDefHideCommand / AgentDefUnhideCommand
         * — Phase 2 of SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md.
         * `listagents` excludes hidden templates by default; only the
         * settings unhide UI passes `include_hidden: true` to see them.
         */
        user_hidden?: number;
        /**
         * Container image to pull when agent_type === "container".
         * Populated from the seed manifest (cli-catalog.ts `containerImage`).
         * Empty string for host-only agents.
         */
        container_image?: string;
        /**
         * Explicit per-agent opt-in (0/1) to the CLI's global (ambient)
         * login when no oauth-class account resolves at spawn. 0 (default)
         * = spawn fails with a visible error instead of silently falling
         * back to ~/.claude. Toggled from the Agent setup modal's Accounts
         * tab. Schema v12 — layer 3 of
         * SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.3.
         */
        use_ambient_login?: number;
        /**
         * Redirects this agent's harness (CLI) at a non-default model vendor
         * backend — e.g. a custom `ANTHROPIC_BASE_URL` for a claude-provider
         * agent. Empty/absent = use the harness's default vendor endpoint.
         * Schema v15. Only settable via `agent.define` today; the human
         * "New Agent"/edit UI (createagent/updateagent) doesn't surface it
         * yet — see `agent_define_core`'s `validate_vendor_base_url`.
         */
        model_vendor_base_url?: string;
        /**
         * Per-agent opt-in: when non-zero, a running Warden Supervisor
         * watcher agent is permitted to auto-continue this agent's session
         * on turn-end (subject to a server-side consecutive-nudge ceiling).
         * 0 (default) = opt-in required, same fail-by-default posture as
         * use_ambient_login. Schema v17. Toggled from the Warden Supervisor
         * panel.
         */
        auto_continue_enabled?: number;
        /**
         * The agent's own dedicated ABF bundle (`Memory.id`). Set once —
         * readonly after creation, same posture as `slug`/`parent_id`
         * (`updateagent` preserves it from the existing row rather than
         * accepting a client-supplied value). Empty string = not yet
         * provisioned (legacy row predating this field). Distinct from an
         * `AgentInstance`'s own `memory_id`, which can still point at a
         * different bundle on purpose for one specific launch. Schema v19 —
         * see ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.1.
         */
        memory_id?: string;
    };

    // ── v6: identity, instance, junction ────────────────────────────────────

    /**
     * Discriminated-union secret reference. Stored as JSON in
     * `IdentityAccount.secret_ref`. The actual secret value is NEVER stored;
     * only how to look it up at launch time. `plaintext_dev` is dev-only.
     */
    type SecretRef =
        | { backend: "env"; env_var: string }
        | { backend: "secrets_manager"; sm_path: string; sm_json_path?: string }
        | { backend: "plaintext_dev"; plaintext_dev: string }
        // Armory API keys: pointer into the OS keychain. Plaintext is
        // never carried here. See specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §7/§12.2.
        | { backend: "keychain"; service: string; account: string }
        // OAuth credentials as a filesystem pointer: the provider CLI reads
        // its tokens from this dir at spawn time; agentmux holds only the
        // path. See SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md and the Rust
        // SecretRef::OAuthConfigDir variant (storage/identities.rs).
        | { backend: "oauth_config_dir"; dir: string };

    type IdentityAccount = {
        id: string;
        name: string;
        provider: string; // "github" | "aws" | "anthropic" | "custom"
        kind: string;     // "pat" | "role" | "api_key" | "env_ref"
        display_name?: string;
        secret_ref: SecretRef;
        /** Free-form per-provider context. Frontend types it by `provider`. */
        context: Record<string, unknown>;
        status?: string; // "unknown" | "ok" | "expired" | "invalid"
        created_at: number;
        updated_at: number;
    };

    type AgentDefinitionIdentity = {
        agent_id: string;
        account_id: string;
        provider: string;
    };

    // ── v7 — Memory bundles ────────────────────────────────────────────

    /** A Memory bundle — the agent's personality and capability stack:
     *  provider/CLI choice, model, system instructions, context files,
     *  MCP servers, skills. The blank singleton represents "vanilla CLI". */
    type Memory = {
        id: string;
        name: string;
        description?: string;
        is_blank?: boolean;
        /** When true this bundle is automatically injected into every agent's
         *  CLAUDE.md at launch (Armory global tier). Managed in the
         *  Identity & Memory hamburger modal. */
        is_global?: boolean;
        provider?: string;            // "claude" | "codex" | "gemini" | ""
        model?: string;
        instructions?: string;
        /** JSON-encoded object of `{ provider_id: content }` — additive,
         *  harness-scoped instruction variants alongside `instructions`
         *  above (which keeps meaning "default"). ABF v0.2 §2.2. */
        instructions_by_provider?: string;
        /** JSON-encoded array of `{ path, content }`. */
        context_files?: string;
        /** JSON-encoded array of MCP server configs. */
        mcp_servers?: string;
        /** JSON-encoded array of skill IDs. */
        skills?: string;
        /** Explicit ordering within the Armory global brain (controls
         *  CLAUDE.md injection order). Only meaningful for is_global bundles;
         *  0 otherwise. Owned by the reorderglobalbrain RPC. */
        sort_order?: number;
        created_at: number;
        updated_at: number;
    };

    // ── Armory Bundle Format (ABF) import, Phase 3 ──────────────────────
    // agentmux-srv/src/server/app_api/bundle.rs — bundle.import.preview /
    // bundle.import.commit. See docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md.

    type BundleImportContextFilePreview = {
        /** Stable selection key (0-based index within this parse) — never
         *  the (possibly truncated) display_path. */
        id: number;
        display_path: string;
        size_bytes: number;
    };

    /** "none" | "name_conflict" | "duplicate_in_bundle". */
    type BundleImportSkillCollision = "none" | "name_conflict" | "duplicate_in_bundle";

    type BundleImportSkillPreview = {
        /** Stable selection key — never the (possibly truncated) slug. */
        source_dir: string;
        slug: string;
        description: string;
        collision: BundleImportSkillCollision;
    };

    type BundleImportMcpServerPreview = {
        /** Stable selection key. */
        source_path: string;
        display: { name: string | null; command: string | null };
    };

    type BundleImportRequirementPreview = {
        id: string;
        provider: string;
        env: string;
        resolved: boolean;
        match_count: number;
    };

    type BundleImportPreviewResponse = {
        name: string;
        description: string;
        instructions_preview: string;
        instructions_truncated: boolean;
        instructions_total_chars: number;
        context_files: BundleImportContextFilePreview[];
        skills: BundleImportSkillPreview[];
        mcp_servers: BundleImportMcpServerPreview[];
        requirements: BundleImportRequirementPreview[];
        warnings: string[];
        warnings_truncated: boolean;
        name_collision: boolean;
        /** Required back at commit as expected_content_digest — proves the
         *  file hasn't changed since preview. */
        content_digest: string;
    };

    type BundleImportCommitResponse = {
        bundle_id: string;
        imported_skill_ids: string[];
        skipped_skills: string[];
        resolved_requirement_ids: string[];
        unresolved_requirements: { id: string; provider: string; env: string; match_count: number }[];
        warnings: string[];
        warnings_truncated: boolean;
    };

    // Mirrors agentmux-srv/src/backend/bundle_validate.rs::{ValidationIssue,
    // ValidationReport}. `field` is one of "instructions_by_provider",
    // "context_files", "mcp_servers", "skills".
    type BundleValidationIssue = {
        severity: "error" | "warning";
        field: string;
        message: string;
    };

    type BundleValidationReport = {
        is_valid: boolean;
        issues: BundleValidationIssue[];
    };

    // ── v1 composable model — standalone MCP Server + Skill primitives ─────
    // Mirrors agentmux-srv/src/backend/storage/mcp_servers.rs::McpServer and
    // skills.rs::Skill (the v1 struct, not the legacy agent-scoped AgentSkill).

    /** A standalone MCP Server primitive. `config` is a JSON-encoded object
     *  (command/args/env for stdio; url/headers for sse) merged into
     *  `.mcp.json` at agent launch. Global servers (`is_global`) are visible
     *  to every agent and cannot be mutated/deleted by agent-scoped RPCs. */
    type McpServer = {
        id: string;
        name: string;
        transport: string; // "stdio" | "sse"
        config: string;
        is_global: boolean;
        created_at: number;
        updated_at: number;
    };

    /** A standalone Skill primitive — an on-demand instruction/knowledge
     *  module, loaded when invoked. Global skills (`is_global`) are visible
     *  to every agent and cannot be mutated/deleted by agent-scoped RPCs. */
    type Skill = {
        id: string;
        name: string;
        trigger: string;
        skill_type: string; // "prompt" | ...
        description: string;
        content: string;
        is_global: boolean;
        created_at: number;
        updated_at: number;
    };

    /** `mcp.list`'s response shape: an McpServer plus whether the requesting
     *  agent specifically holds the bind ref (as opposed to just being able
     *  to see it because it's global). Only `mcp.list` (agent-scoped) carries
     *  this — `mcp.get`/`mcp.upsert`/`mcp.catalog.list` return bare McpServer. */
    type McpServerListItem = McpServer & { bound_to_agent: boolean };

    /** `mcp.probe`/`mcp.catalog.probe`'s response shape — a protocol-level
     *  health check (SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md
     *  §4.4). "connected" means the MCP handshake succeeded, not that any
     *  external app/prerequisite the server itself depends on is available. */
    type McpProbeResult = {
        status: "connected" | "unreachable" | "handshake_failed" | "invalid_config";
        tool_count: number | null;
        server_name: string | null;
        server_version: string | null;
        error: string | null;
    };

    /** `skill.list`'s response shape — see McpServerListItem. */
    type SkillListItem = Skill & { bound_to_agent: boolean };

    /** `mcp.catalog.list`'s response shape: an McpServer plus how many
     *  agents currently bind it — the Armory catalog's "used by N agents"
     *  count (#1960 gap #2). */
    type McpServerCatalogItem = McpServer & { bound_count: number };

    /** `skill.catalog.list`'s response shape — see McpServerCatalogItem. */
    type SkillCatalogItem = Skill & { bound_count: number };

    /** Drone pane (issue #753 Phase 1). Mirrors the Rust types in
     *  agentmux-srv/src/drone/types.rs. */
    type DroneDefinition = {
        id: string;
        name: string;
        description: string;
        graph: { nodes: DroneFlowNode[]; edges: DroneFlowEdge[] };
        viewport: { x: number; y: number; zoom: number };
        created_at: number;
        updated_at: number;
    };

    type DroneFlowNode = {
        id: string;
        position: { x: number; y: number };
        data: Record<string, unknown> & { kind: string };
        type?: string;
    };

    type DroneFlowEdge = {
        id: string;
        source: string;
        target: string;
        sourceHandle?: string;
        targetHandle?: string;
    };

    type DroneRun = {
        id: string;
        drone_id: string;
        status: string;
        started_at: number;
        ended_at: number;
        block_states: Record<string, DroneBlockState>;
        output: string;
        error: string;
    };

    type DroneBlockState = {
        status: "pending" | "running" | "done" | "error" | "skipped";
        output?: unknown;
        error?: string;
        started_at?: number;
        completed_at?: number;
    };

    type AgentInstanceStatus = "running" | "paused" | "stopped" | "crashed" | "detached";

    type GitHubContext = {
        repo: string; // "owner/repo"
        pr_number?: number;
        branch?: string;
        issue_number?: number;
        workflow_run_id?: number;
    };

    type AgentInstance = {
        id: string;
        definition_id: string;
        parent_instance_id?: string;
        block_id?: string;
        session_id?: string;
        status: string; // AgentInstanceStatus
        /** JSON-encoded GitHubContext, or empty string. */
        github_context?: string;
        started_at: number;
        ended_at?: number;
        created_at: number;
        /** v7/v11 — legacy Identity-bundle id; db_identity_bundles was
         *  dropped in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md.
         *  Vestigial opaque pass-through now — credential resolution and
         *  display names both go through db_agent_identity_links/db_accounts.
         *  Empty string = ambient creds. */
        identity_id?: string;
        /** v7/v11 — FK to db_bundles. Empty string = blank singleton. */
        memory_id?: string;
        /** v8 — user-chosen instance name (AGENTMUX_AGENT_ID). */
        instance_name?: string;
        /** v8 — absolute working directory from allocate_agent_workdir. */
        working_directory?: string;
        /** v8 — soft-delete flag for the "Forget agent" affordance. */
        display_hidden?: boolean;
    };

    // ────────────────────────────────────────────────────────────────
    // Unified agent types (Drone Phase 1.5, see
    // docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md). Shared
    // between the agent pane and the drone Agent block. Mirror
    // of agentmux-srv/src/agents/types.rs — camelCase via serde
    // rename_all so the field shapes match without translation.
    // ────────────────────────────────────────────────────────────────

    /** Identifies "which agent" — same shape for launch modal + drone Agent block. */
    type AgentRef = {
        identityId?: string;
        memoryId?: string;
        instanceName?: string;
        workingDirectory?: string;
    };

    type AgentTask = {
        prompt: string;
        context?: Record<string, unknown>;
        maxTurns?: number;
    };

    type TokenCounts = {
        input: number;
        output: number;
        cacheCreation: number;
        cacheRead: number;
    };

    type AgentTurn = {
        role: "user" | "assistant" | "tool_result";
        content: unknown;
        timestampMs: number;
    };

    type AgentEvent =
        | { type: "assistant_text"; delta: string }
        | { type: "tool_use"; toolUseId: string; tool: string; input: unknown }
        | { type: "tool_result"; toolUseId: string; output: unknown; isError: boolean }
        | { type: "cost"; costUsd: number; tokens: TokenCounts }
        | { type: "done"; response: string; transcript: AgentTurn[] }
        | { type: "error"; message: string }
        /**
         * Context compaction completed. Sourced from the CLI's own
         * `system`/`compact_boundary` stream-json frame — real counts, not
         * inferred. Mirror of `agentmux-srv/src/agents/types.rs`'s
         * `AgentEvent::CompactionBoundary` (`CompactionTrigger` is
         * `#[serde(rename_all = "snake_case")]`). See
         * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md
         * (Codex P2, PR #2378 round 4: this mirror was missing the variant
         * entirely, leaving typed consumers of this wire union unable to
         * represent an event the backend actually emits).
         */
        | {
              type: "compaction_boundary";
              trigger: "auto" | "manual";
              preTokens: number;
              postTokens: number;
              cumulativeDroppedTokens: number;
              durationMs: number;
          };

    /**
     * Wire shape of one pre-launch OAuth session's current status.
     * Mirror of `agentmux-srv/src/identity/auth_session.rs::AuthSessionStatus`
     * (`#[serde(tag = "status", rename_all = "kebab-case",
     * rename_all_fields = "camelCase")]`). Spec:
     * `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` §7.
     */
    type AuthSessionStatus =
        | { status: "pending" }
        | { status: "url-available"; authUrl: string }
        | { status: "code-emitted"; deviceCode: string; verificationUrl: string }
        | { status: "success"; bundleId: string; email: string | null; accountId?: string }
        | { status: "failed"; error: string };

    type AgentRunResult = {
        response: string;
        tokens: TokenCounts;
        costUsd: number;
        transcript: AgentTurn[];
    };

    /** v8 — one row of the launch modal's "Continue agent" dropdown.
     * Server-side join across instance + definition + bundle tables. */
    type NamedAgentRow = {
        instance_id: string;
        instance_name: string;
        definition_id: string;
        definition_name: string;
        provider: string;
        working_directory: string;
        identity_id: string;
        identity_name: string;
        memory_id: string;
        memory_name: string;
        started_at: number;
        ended_at: number;
        status: string;
        block_id_hint: string;
    };

    /** Cascade follow-up (2026-05-23) — one row of the AgentPicker's
     * "Recent sessions" list. Mirrors NamedAgentRow + adds a preview
     * (first user message text) and node count read from the filestore
     * `output.state.json` snapshot. has_snapshot: false means the
     * pane never wrote a snapshot (legacy / pre-persistence row); the
     * row still surfaces so the user can reattach. */
    type RecentSessionRow = {
        instance_id: string;
        instance_name: string;
        definition_id: string;
        definition_name: string;
        provider: string;
        /** Custom model vendor base URL override, mirrored from
         *  AgentDefinition.model_vendor_base_url. Empty means the harness
         *  talks to its own default vendor. See providers/catalog.ts's
         *  resolveEffectiveVendor. */
        model_vendor_base_url?: string;
        working_directory: string;
        identity_id: string;
        identity_name: string;
        memory_id: string;
        memory_name: string;
        block_id_hint: string;
        /**
         * CLI-emitted session id captured during the prior run. Empty
         * when the row predates the capture or the CLI didn't emit a
         * session id. Used by the picker reattach to populate
         * `agent:sessionid` on the new block's meta so the spawned
         * subprocess gets a real `--resume <sid>` on the FIRST turn.
         */
        session_id?: string;
        preview: string;
        node_count: number;
        last_active_at: number;
        has_snapshot: boolean;
        /** When the agent definition was first created (ms epoch). */
        agent_created_at: number;
        /** When this instance was most recently launched (ms epoch). */
        started_at: number;
        /** "host" or "container" — drives the runtime badge in the My Agents list. */
        agent_type?: string;
    };

    // AgentContent
    type AgentContent = {
        agent_id: string;
        content_type: string;
        content: string;
        updated_at: number;
    };

    // CommandCreateAgentDefinitionData
    type CommandCreateAgentDefinitionData = {
        name: string;
        icon: string;
        provider: string;
        description: string;
        working_directory?: string;
        shell?: string;
        provider_flags?: string;
        auto_start?: number;
        restart_on_crash?: number;
        idle_timeout_minutes?: number;
        agent_type?: string;
        environment?: string;
        agent_bus_id?: string;
    };

    // CommandUpdateAgentDefinitionData
    type CommandUpdateAgentDefinitionData = {
        id: string;
        name: string;
        icon: string;
        provider: string;
        description: string;
        working_directory?: string;
        shell?: string;
        provider_flags?: string;
        auto_start?: number;
        restart_on_crash?: number;
        idle_timeout_minutes?: number;
        agent_type?: string;
        environment?: string;
        agent_bus_id?: string;
        /** JSON-encoded per-provider account refs. See AgentDefinition.accounts. */
        accounts?: string;
        /**
         * Explicit ambient-login opt-in (0/1). Omit to preserve the stored
         * value — the backend treats absence as "no change". See
         * AgentDefinition.use_ambient_login.
         */
        use_ambient_login?: number;
        /**
         * Per-agent opt-in letting a Warden Supervisor watcher agent
         * auto-continue this agent's session on turn-end (0/1). Omit to
         * preserve the stored value. See AgentDefinition.auto_continue_enabled.
         */
        auto_continue_enabled?: number;
        /**
         * Custom model vendor base URL override. Omit to preserve the
         * stored value; "" explicitly clears it back to the harness's
         * default vendor endpoint. See AgentDefinition.model_vendor_base_url.
         */
        model_vendor_base_url?: string;
    };

    // CommandDeleteAgentDefinitionData
    type CommandDeleteAgentDefinitionData = {
        id: string;
    };

    // CommandGetAgentContentData
    type CommandGetAgentContentData = {
        agent_id: string;
        content_type: string;
    };

    // CommandSetAgentContentData
    type CommandSetAgentContentData = {
        agent_id: string;
        content_type: string;
        content: string;
    };

    // CommandGetAllAgentContentData
    type CommandGetAllAgentContentData = {
        agent_id: string;
    };

    // AgentSkill
    type AgentSkill = {
        id: string;
        agent_id: string;
        name: string;
        trigger: string;
        skill_type: string;
        description: string;
        content: string;
        created_at: number;
    };

    // CommandListAgentSkillsData
    type CommandListAgentSkillsData = {
        agent_id: string;
    };

    // CommandCreateAgentSkillData
    type CommandCreateAgentSkillData = {
        agent_id: string;
        name: string;
        trigger?: string;
        skill_type?: string;
        description?: string;
        content?: string;
    };

    // CommandUpdateAgentSkillData
    type CommandUpdateAgentSkillData = {
        id: string;
        name: string;
        trigger?: string;
        skill_type?: string;
        description?: string;
        content?: string;
    };

    // CommandDeleteAgentSkillData
    type CommandDeleteAgentSkillData = {
        id: string;
    };

    // AgentHistory
    type AgentHistory = {
        id: number;
        agent_id: string;
        session_date: string;
        entry: string;
        timestamp: number;
    };

    // CommandAppendAgentHistoryData
    type CommandAppendAgentHistoryData = {
        agent_id: string;
        entry: string;
    };

    // CommandListAgentHistoryData
    type CommandListAgentHistoryData = {
        agent_id: string;
        session_date?: string;
        limit?: number;
        offset?: number;
    };

    // CommandSearchAgentHistoryData
    type CommandSearchAgentHistoryData = {
        agent_id: string;
        query: string;
        limit?: number;
    };

    // CommandImportAgentFromClawData
    type CommandImportAgentFromClawData = {
        workspace_path: string;
        agent_name: string;
    };

    // wshrpc.CommandDeleteBlockData
    type CommandDeleteBlockData = {
        blockid: string;
    };

    // wshrpc.CommandDeleteFileData
    type CommandDeleteFileData = {
        path: string;
        recursive: boolean;
    };

    // wshrpc.CommandDisposeData
    type CommandDisposeData = {
        routeid: string;
    };

    // wshrpc.CommandEventReadHistoryData
    type CommandEventReadHistoryData = {
        event: string;
        scope: string;
        maxitems: number;
    };

    // wshrpc.CommandFileCopyData
    type CommandFileCopyData = {
        srcuri: string;
        desturi: string;
        opts?: FileCopyOpts;
    };

    // wshrpc.CommandGetMetaData
    type CommandGetMetaData = {
        oref: ORef;
    };

    // wshrpc.CommandGetRTInfoData
    type CommandGetRTInfoData = {
        oref: ORef;
    };

    // wshrpc.CommandGetWaveAIChatData
    type CommandGetWaveAIChatData = {
        chatid: string;
    };

    // wshrpc.CommandMessageData
    type CommandMessageData = {
        oref: ORef;
        message: string;
    };

    // wshrpc.CommandRemoteListEntriesData
    type CommandRemoteListEntriesData = {
        path: string;
        opts?: FileListOpts;
    };

    // wshrpc.CommandRemoteListEntriesRtnData
    type CommandRemoteListEntriesRtnData = {
        fileinfo?: FileInfo[];
    };

    // wshrpc.CommandRemoteStreamFileData
    type CommandRemoteStreamFileData = {
        path: string;
        byterange?: string;
    };

    // wshrpc.CommandRemoteStreamTarData
    type CommandRemoteStreamTarData = {
        path: string;
        opts?: FileCopyOpts;
    };

    // wshrpc.CommandResolveIdsData
    type CommandResolveIdsData = {
        blockid: string;
        ids: string[];
    };

    // wshrpc.CommandResolveIdsRtnData
    type CommandResolveIdsRtnData = {
        resolvedids: {[key: string]: ORef};
    };

    // wshrpc.CommandSetMetaData
    type CommandSetMetaData = {
        oref: ORef;
        meta: MetaType;
    };

    // wshrpc.CommandSetRTInfoData
    type CommandSetRTInfoData = {
        oref: ORef;
        data: ObjRTInfo;
    };

    // wshrpc.CommandTermGetScrollbackLinesData
    type CommandTermGetScrollbackLinesData = {
        linestart: number;
        lineend: number;
    };

    // wshrpc.CommandTermGetScrollbackLinesRtnData
    type CommandTermGetScrollbackLinesRtnData = {
        totallines: number;
        linestart: number;
        lines: string[];
        lastupdated: number;
    };

    // wshrpc.CommandVarData
    type CommandVarData = {
        key: string;
        val?: string;
        remove?: boolean;
        zoneid: string;
        filename: string;
    };

    // wshrpc.CommandVarResponseData
    type CommandVarResponseData = {
        key: string;
        val: string;
        exists: boolean;
    };

    // wshrpc.CommandWaitForRouteData
    type CommandWaitForRouteData = {
        routeid: string;
        waitms: number;
    };

    // wshrpc.CommandWaveAIAddContextData
    type CommandWaveAIAddContextData = {
        files?: AIAttachedFile[];
        text?: string;
        submit?: boolean;
        newchat?: boolean;
    };

    // wshrpc.CommandWaveAIToolApproveData
    type CommandWaveAIToolApproveData = {
        toolcallid: string;
        keepalive?: boolean;
        approval?: string;
    };

    // wshrpc.CommandWebSelectorData
    type CommandWebSelectorData = {
        workspaceid: string;
        blockid: string;
        tabid: string;
        selector: string;
        opts?: WebSelectorOpts;
    };

    // wconfig.ConfigError
    type ConfigError = {
        file: string;
        err: string;
    };

    // wshrpc.ConnConfigRequest
    type ConnConfigRequest = {
        host: string;
        metamaptype: MetaType;
    };

    // wshrpc.ConnExtData
    type ConnExtData = {
        connname: string;
        logblockid?: string;
    };

    // wconfig.ConnKeywords
    type ConnKeywords = {
        "conn:shellpath"?: string;
        "conn:ignoresshconfig"?: boolean;
        "display:hidden"?: boolean;
        "display:order"?: number;
        "term:*"?: boolean;
        "term:fontsize"?: number;
        "term:fontfamily"?: string;
        "term:zoom"?: number;
        "term:theme"?: string;
        "cmd:env"?: {[key: string]: string};
        "cmd:initscript"?: string;
        "cmd:initscript.sh"?: string;
        "cmd:initscript.bash"?: string;
        "cmd:initscript.zsh"?: string;
        "cmd:initscript.pwsh"?: string;
        "cmd:initscript.fish"?: string;
        "ssh:user"?: string;
        "ssh:hostname"?: string;
        "ssh:port"?: string;
        "ssh:identityfile"?: string[];
        "ssh:batchmode"?: boolean;
        "ssh:pubkeyauthentication"?: boolean;
        "ssh:passwordauthentication"?: boolean;
        "ssh:kbdinteractiveauthentication"?: boolean;
        "ssh:preferredauthentications"?: string[];
        "ssh:addkeystoagent"?: boolean;
        "ssh:identityagent"?: string;
        "ssh:identitiesonly"?: boolean;
        "ssh:proxyjump"?: string[];
        "ssh:userknownhostsfile"?: string[];
        "ssh:globalknownhostsfile"?: string[];
    };

    // wshrpc.ConnRequest
    type ConnRequest = {
        host: string;
        keywords?: ConnKeywords;
        logblockid?: string;
    };

    // wshrpc.ConnStatus
    type ConnStatus = {
        status: string;
        connection: string;
        connected: boolean;
        hasconnected: boolean;
        activeconnnum: number;
        error?: string;
    };

    // wshrpc.CpuDataRequest
    type CpuDataRequest = {
        id: string;
        count: number;
    };

    // vdom.DomRect
    type DomRect = {
        top: number;
        left: number;
        right: number;
        bottom: number;
        width: number;
        height: number;
    };

    // wshrpc.FetchSuggestionsData
    type FetchSuggestionsData = {
        suggestiontype: string;
        query: string;
        widgetid: string;
        reqnum: number;
        "file:cwd"?: string;
        "file:dironly"?: boolean;
        "file:connection"?: string;
    };

    // wshrpc.FetchSuggestionsResponse
    type FetchSuggestionsResponse = {
        reqnum: number;
        suggestions: SuggestionType[];
    };

    // wshrpc.FileCopyOpts
    type FileCopyOpts = {
        overwrite?: boolean;
        recursive?: boolean;
        merge?: boolean;
        timeout?: number;
    };

    // wshrpc.FileData
    type FileData = {
        info?: FileInfo;
        data64?: string;
        entries?: FileInfo[];
        at?: FileDataAt;
    };

    // wshrpc.FileDataAt
    type FileDataAt = {
        offset: number;
        size?: number;
    };

    // waveobj.FileDef
    type FileDef = {
        content?: string;
        meta?: {[key: string]: any};
    };

    // wshrpc.FileInfo
    type FileInfo = {
        path: string;
        dir?: string;
        name?: string;
        notfound?: boolean;
        opts?: FileOpts;
        size?: number;
        meta?: {[key: string]: any};
        mode?: number;
        modestr?: string;
        modtime?: number;
        isdir?: boolean;
        supportsmkdir?: boolean;
        mimetype?: string;
        readonly?: boolean;
    };

    // wshrpc.FileListData
    type FileListData = {
        path: string;
        opts?: FileListOpts;
    };

    // wshrpc.FileListOpts
    type FileListOpts = {
        all?: boolean;
        offset?: number;
        limit?: number;
    };

    // wshrpc.FileOpts
    type FileOpts = {
        maxsize?: number;
        circular?: boolean;
        ijson?: boolean;
        ijsonbudget?: number;
        truncate?: boolean;
        append?: boolean;
    };

    // wshrpc.FileShareCapability
    type FileShareCapability = {
        canappend: boolean;
        canmkdir: boolean;
    };

    // wconfig.FullConfigType
    type FullConfigType = {
        settings: SettingsType;
        mimetypes: {[key: string]: MimeTypeConfigType};
        defaultwidgets: {[key: string]: WidgetConfigType};
        widgets: {[key: string]: WidgetConfigType};
        presets: {[key: string]: MetaType};
        termthemes: {[key: string]: TermThemeType};
        connections: {[key: string]: ConnKeywords};
        bookmarks: {[key: string]: WebBookmark};
        configerrors: ConfigError[];
    };

    // waveobj.LayoutActionData
    type LayoutActionData = {
        actiontype: string;
        actionid: string;
        blockid: string;
        nodesize?: number;
        nodesizefraction?: number;
        indexarr?: number[];
        focused: boolean;
        magnified: boolean;
        ephemeral: boolean;
        targetblockid?: string;
        position?: string;
    };

    // waveobj.LayoutState
    type LayoutState = WaveObj & {
        rootnode?: any;
        magnifiednodeid?: string;
        focusednodeid?: string;
        leaforder?: LeafOrderEntry[];
        pendingbackendactions?: LayoutActionData[];
    };

    // waveobj.LeafOrderEntry
    type LeafOrderEntry = {
        nodeid: string;
        blockid: string;
    };

    // waveobj.MetaTSType
    type MetaType = {
        view?: string;
        controller?: string;
        file?: string;
        url?: string;
        pinnedurl?: string;
        connection?: string;
        edit?: boolean;
        history?: string[];
        "history:forward"?: string[];
        "display:name"?: string;
        "display:order"?: number;
        icon?: string;
        "icon:color"?: string;
        "frame:*"?: boolean;
        frame?: boolean;
        "frame:bordercolor"?: string;
        "frame:activebordercolor"?: string;
        "frame:title"?: string;
        "frame:icon"?: string;
        "frame:text"?: string;
        // Tab accent color (frontend/app/tab/tab.tsx, tabbar.tsx). Was missing
        // here despite being a real, actively-set meta key — #859.
        "tab:color"?: string | null;
        "tab:color-initialized"?: boolean;
        "cmd:*"?: boolean;
        cmd?: string;
        "cmd:interactive"?: boolean;
        "cmd:login"?: boolean;
        "cmd:runonstart"?: boolean;
        "cmd:clearonstart"?: boolean;
        "cmd:runonce"?: boolean;
        "cmd:closeonexit"?: boolean;
        "cmd:closeonexitforce"?: boolean;
        "cmd:closeonexitdelay"?: number;
        "cmd:nowsh"?: boolean;
        "cmd:args"?: string[];
        "cmd:shell"?: boolean;
        "cmd:allowconnchange"?: boolean;
        "cmd:jwt"?: boolean;
        "cmd:env"?: {[key: string]: string};
        "cmd:cwd"?: string;
        "cmd:initscript"?: string;
        "cmd:initscript.sh"?: string;
        "cmd:initscript.bash"?: string;
        "cmd:initscript.zsh"?: string;
        "cmd:initscript.pwsh"?: string;
        "cmd:initscript.fish"?: string;
        "ai:*"?: boolean;
        "ai:preset"?: string;
        "ai:apitype"?: string;
        "ai:baseurl"?: string;
        "ai:apitoken"?: string;
        "ai:name"?: string;
        "ai:model"?: string;
        "ai:orgid"?: string;
        "ai:apiversion"?: string;
        "ai:maxtokens"?: number;
        "ai:timeoutms"?: number;
        "editor:*"?: boolean;
        "editor:minimapenabled"?: boolean;
        "editor:stickyscrollenabled"?: boolean;
        "editor:wordwrap"?: boolean;
        "editor:fontsize"?: number;
        "graph:*"?: boolean;
        "graph:numpoints"?: number;
        "graph:metrics"?: string[];
        "sysinfo:type"?: string;
        "bg:*"?: boolean;
        bg?: string;
        "bg:opacity"?: number;
        "bg:blendmode"?: string;
        "bg:bordercolor"?: string;
        "bg:activebordercolor"?: string;
        "waveai:panelopen"?: boolean;
        "waveai:panelwidth"?: number;
        "waveai:model"?: string;
        "waveai:chatid"?: string;
        "waveai:widgetcontext"?: boolean;
        "term:*"?: boolean;
        "term:fontsize"?: number;
        "term:fontfamily"?: string;
        "term:zoom"?: number;
        "term:mode"?: string;
        "term:theme"?: string;
        "help:zoom"?: number;
        /** Absolute path to a file or directory the Media pane is pointed
         *  at. See docs/specs/SPEC_MEDIA_PANE_2026_07_26.md. */
        "media:path"?: string;
        "term:localshellpath"?: string;
        "term:localshellopts"?: string[];
        "term:scrollback"?: number;
        "term:transparency"?: number;
        "term:allowbracketedpaste"?: boolean;
        "term:shiftenternewline"?: boolean;
        "term:conndebug"?: string;
        "markdown:fontsize"?: number;
        "markdown:fixedfontsize"?: number;
        "onboarding:githubstar"?: boolean;
        "onboarding:lastversion"?: string;
        count?: number;
        "widget:order"?: string[];
        "agent:*"?: boolean;
        agentId?: string;
        /** v6 — DB row id of the AgentInstance currently bound to this pane. */
        agentInstanceId?: string;
        agentName?: string;
        agentIcon?: string;
        agentMode?: string;
        agentProvider?: string;
        agentCliPath?: string;
        agentCliArgs?: string[];
        agentOutputFormat?: string;
        agentBinDir?: string;
        "agent:resume_flag"?: string;
        "agent:resume_strategy"?: "none" | "flag" | "codex-exec";
        "agent:session_id_field"?: string;
        "agent:sessionid"?: string;
        /** Last classified agent failure; set on error exit, cleared on clean exit. */
        "agent:last_failure"?: AgentFailure;
        "agent:runtime"?: {
            permissionMode?: string;
            model?: string | null;
            effort?: string | null;
        };
        "session:start_ts_ms"?: number;
        "session:last_activity_ms"?: number;
        "session:line_count"?: number;
        "subagent:*"?: boolean;
        "subagent:id"?: string;
        "subagent:slug"?: string;
        "subagent:parent"?: string;
        "subagent:session"?: string;
    };

    // tsgenmeta.MethodMeta
    type MethodMeta = {
        Desc: string;
        ArgNames: string[];
        ReturnDesc: string;
    };

    // wconfig.MimeTypeConfigType
    type MimeTypeConfigType = {
        icon: string;
        color: string;
    };

    // waveobj.ORef
    type ORef = string;

    // waveobj.ObjRTInfo
    type ObjRTInfo = {
        "cmd:hascurcwd"?: boolean;
        "shell:state"?: string;
        "shell:type"?: string;
        "shell:version"?: string;
        "shell:uname"?: string;
        "shell:inputempty"?: boolean;
        "shell:lastcmd"?: string;
        "shell:lastcmdexitcode"?: number;
    };

    // iochantypes.Packet
    type Packet = {
        Data: string;
        Checksum: string;
    };

    // wshrpc.PathCommandData
    type PathCommandData = {
        pathtype: string;
        open: boolean;
        openexternal: boolean;
        tabid: string;
    };

    // waveobj.Point
    type Point = {
        x: number;
        y: number;
    };

    // uctypes.RateLimitInfo
    type RateLimitInfo = {
        req: number;
        reqlimit: number;
        preq: number;
        preqlimit: number;
        resetepoch: number;
        unknown?: boolean;
    };

    // wshrpc.RemoteInfo
    type RemoteInfo = {
        clientarch: string;
        clientos: string;
        clientversion: string;
        shell: string;
    };

    // wshutil.RpcMessage
    type RpcMessage = {
        command?: string;
        reqid?: string;
        resid?: string;
        timeout?: number;
        route?: string;
        authtoken?: string;
        source?: string;
        cont?: boolean;
        cancel?: boolean;
        error?: string;
        datatype?: string;
        data?: any;
    };

    // wshrpc.RpcOpts
    type RpcOpts = {
        timeout?: number;
        noresponse?: boolean;
        route?: string;
    };

    // waveobj.RuntimeOpts
    type RuntimeOpts = {
        termsize?: TermSize;
        winsize?: WinSize;
    };

    // webcmd.SetBlockTermSizeWSCommand
    type SetBlockTermSizeWSCommand = {
        wscommand: "setblocktermsize";
        blockid: string;
        termsize: TermSize;
    };

    // wconfig.SettingsType
    type SettingsType = {
        "app:*"?: boolean;
        "app:globalhotkey"?: string;
        "app:dismissarchitecturewarning"?: boolean;
        "app:defaultnewblock"?: string;
        "app:showoverlayblocknums"?: boolean;
        "term:*"?: boolean;
        "term:fontsize"?: number;
        "term:fontfamily"?: string;
        "term:theme"?: string;
        "term:disablewebgl"?: boolean;
        "term:localshellpath"?: string;
        "term:localshellopts"?: string[];
        "term:scrollback"?: number;
        "term:copyonselect"?: boolean;
        "term:transparency"?: number;
        "term:allowbracketedpaste"?: boolean;
        "term:shiftenternewline"?: boolean;
        "term:predictiveecho"?: boolean;
        "term:predictiveecho:thresholdms"?: number;
        "cmd:env"?: {[key: string]: string};
        "blockheader:*"?: boolean;
        "blockheader:showblockids"?: boolean;
        "preview:showhiddenfiles"?: boolean;
        "tab:preset"?: string;
        "tab:skipcloseconfirm"?: boolean;
        "widget:*"?: boolean;
        "widget:showhelp"?: boolean;
        "widget:icononly"?: boolean;
        "window:*"?: boolean;
        "window:transparent"?: boolean;
        "window:blur"?: boolean;
        "window:opacity"?: number;
        "window:bgcolor"?: string;
        "window:reducedmotion"?: boolean;
        "window:tilegapsize"?: number;
        "window:showmenubar"?: boolean;
        "window:nativetitlebar"?: boolean;
        "window:disablehardwareacceleration"?: boolean;
        "window:maxtabcachesize"?: number;
        "window:magnifiedblockopacity"?: number;
        "window:magnifiedblocksize"?: number;
        "window:magnifiedblockblurprimarypx"?: number;
        "window:magnifiedblockblursecondarypx"?: number;
        "window:confirmclose"?: boolean;
        "window:savelastwindow"?: boolean;
        "window:dimensions"?: string;
        "window:zoom"?: number;
        "window:theme"?: string;
        "telemetry:*"?: boolean;
        "telemetry:enabled"?: boolean;
        "telemetry:interval"?: number;
        "telemetry:numpoints"?: number;
        "conn:*"?: boolean;
        "conn:askbeforewshinstall"?: boolean;
        "conn:wshenabled"?: boolean;
        "network:lan_discovery"?: boolean;
        "voice:enabled"?: boolean;
        "voice:engine"?: string;
        "voice:groqApiKey"?: string;
        "voice:whisperCliPath"?: string;
        "voice:whisperModel"?: string;
        "voice:whisperModelPath"?: string;
        "notify:*"?: boolean;
        "notify:sounds:enabled"?: boolean;
        "notify:sounds:volume"?: number;
        "notify:sounds:suppresswhenfocused"?: boolean;
        "notify:sound:agent.turn.complete"?: boolean;
        "notify:sound:agent.turn.error"?: boolean;
        "notify:sound:agent.turn.interrupted"?: boolean;
        "notify:sound:agent.message.accepted"?: boolean;
        "notify:sound:agent.message.rejected"?: boolean;
        "notify:sound:agent.waiting.for.input"?: boolean;
        "notify:sounds:waiting:volume"?: number;
        "notify:tooltones:enabled"?: boolean;
        "notify:tooltones:volume"?: number;
        "notify:tooltones:scope"?: "all" | "focused";
        "dnd:enabled"?: boolean;
        "dnd:concurrency"?: number;
        "dnd:agentinserttoken"?: boolean;
    };

    // waveobj.StickerClickOptsType
    type StickerClickOptsType = {
        sendinput?: string;
        createblock?: BlockDef;
    };

    // waveobj.StickerDisplayOptsType
    type StickerDisplayOptsType = {
        icon: string;
        imgsrc: string;
        svgblob?: string;
    };

    // waveobj.StickerType
    type StickerType = {
        stickertype: string;
        style: {[key: string]: any};
        clickopts?: StickerClickOptsType;
        display: StickerDisplayOptsType;
    };

    // wps.SubscriptionRequest
    type SubscriptionRequest = {
        event: string;
        scopes?: string[];
        allscopes?: boolean;
    };

    // wshrpc.SuggestionType
    type SuggestionType = {
        type: string;
        suggestionid: string;
        display: string;
        subtext?: string;
        icon?: string;
        iconcolor?: string;
        iconsrc?: string;
        matchpos?: number[];
        submatchpos?: number[];
        score?: number;
        "file:mimetype"?: string;
        "file:path"?: string;
        "file:name"?: string;
        "url:url"?: string;
    };

    // telemetrydata.TEvent
    type TEvent = {
        uuid?: string;
        ts?: number;
        tslocal?: string;
        event: string;
        props: TEventProps;
    };

    // telemetrydata.TEventProps
    type TEventProps = {
        "client:arch"?: string;
        "client:version"?: string;
        "client:initial_version"?: string;
        "client:buildtime"?: string;
        "client:osrelease"?: string;
        "client:isdev"?: boolean;
        "autoupdate:channel"?: string;
        "autoupdate:enabled"?: boolean;
        "localshell:type"?: string;
        "localshell:version"?: string;
        "loc:countrycode"?: string;
        "loc:regioncode"?: string;
        "settings:customwidgets"?: number;
        "settings:customaipresets"?: number;
        "settings:customsettings"?: number;
        "activity:activeminutes"?: number;
        "activity:fgminutes"?: number;
        "activity:openminutes"?: number;
        "activity:waveaiactiveminutes"?: number;
        "activity:waveaifgminutes"?: number;
        "app:firstday"?: boolean;
        "app:firstlaunch"?: boolean;
        "action:initiator"?: "keyboard" | "mouse";
        "debug:panictype"?: string;
        "block:view"?: string;
        "ai:backendtype"?: string;
        "ai:local"?: boolean;
        "wsh:cmd"?: string;
        "wsh:haderror"?: boolean;
        "conn:conntype"?: string;
        "onboarding:feature"?: "waveai" | "magnify" | "wsh";
        "onboarding:version"?: string;
        "onboarding:githubstar"?: "already" | "star" | "later";
        "display:height"?: number;
        "display:width"?: number;
        "display:dpr"?: number;
        "display:count"?: number;
        "display:all"?: any;
        "count:blocks"?: number;
        "count:tabs"?: number;
        "count:windows"?: number;
        "count:workspaces"?: number;
        "count:sshconn"?: number;
        "count:wslconn"?: number;
        "count:views"?: {[key: string]: number};
        "waveai:apitype"?: string;
        "waveai:model"?: string;
        "waveai:inputtokens"?: number;
        "waveai:outputtokens"?: number;
        "waveai:nativewebsearchcount"?: number;
        "waveai:requestcount"?: number;
        "waveai:toolusecount"?: number;
        "waveai:tooluseerrorcount"?: number;
        "waveai:tooldetail"?: {[key: string]: number};
        "waveai:premiumreq"?: number;
        "waveai:proxyreq"?: number;
        "waveai:haderror"?: boolean;
        "waveai:imagecount"?: number;
        "waveai:pdfcount"?: number;
        "waveai:textdoccount"?: number;
        "waveai:textlen"?: number;
        "waveai:firstbytems"?: number;
        "waveai:requestdurms"?: number;
        "waveai:widgetaccess"?: boolean;
        "waveai:feedback"?: "good" | "bad";
        $set?: TEventUserProps;
        $set_once?: TEventUserProps;
    };

    // telemetrydata.TEventUserProps
    type TEventUserProps = {
        "client:arch"?: string;
        "client:version"?: string;
        "client:initial_version"?: string;
        "client:buildtime"?: string;
        "client:osrelease"?: string;
        "client:isdev"?: boolean;
        "autoupdate:channel"?: string;
        "autoupdate:enabled"?: boolean;
        "localshell:type"?: string;
        "localshell:version"?: string;
        "loc:countrycode"?: string;
        "loc:regioncode"?: string;
        "settings:customwidgets"?: number;
        "settings:customaipresets"?: number;
        "settings:customsettings"?: number;
    };

    // waveobj.Tab
    type Tab = WaveObj & {
        name: string;
        layoutstate: string;
        blockids: string[];
    };

    // waveobj.TermSize
    type TermSize = {
        rows: number;
        cols: number;
    };

    // wconfig.TermThemeType
    type TermThemeType = {
        "display:name": string;
        "display:order": number;
        black: string;
        red: string;
        green: string;
        yellow: string;
        blue: string;
        magenta: string;
        cyan: string;
        white: string;
        brightBlack: string;
        brightRed: string;
        brightGreen: string;
        brightYellow: string;
        brightBlue: string;
        brightMagenta: string;
        brightCyan: string;
        brightWhite: string;
        gray: string;
        cmdtext: string;
        foreground: string;
        selectionBackground: string;
        background: string;
        cursor: string;
    };

    // wshrpc.TimeSeriesData
    type TimeSeriesData = {
        ts: number;
        values: {[key: string]: number};
    };

    // uctypes.UIChat
    type UIChat = {
        chatid: string;
        apitype: string;
        model: string;
        apiversion: string;
        messages: UIMessage[];
    };

    // waveobj.UIContext
    type UIContext = {
        windowid: string;
        activetabid: string;
    };

    // uctypes.UIMessage
    type UIMessage = {
        id: string;
        role: string;
        metadata?: any;
        parts?: UIMessagePart[];
    };

    // uctypes.UIMessagePart
    type UIMessagePart = {
        type: string;
        text?: string;
        state?: string;
        toolCallId?: string;
        input?: any;
        output?: any;
        errorText?: string;
        providerExecuted?: boolean;
        sourceId?: string;
        url?: string;
        title?: string;
        filename?: string;
        mediaType?: string;
        id?: string;
        data?: any;
        providerMetadata?: {[key: string]: any};
    };

    // userinput.UserInputRequest
    type UserInputRequest = {
        requestid: string;
        querytext: string;
        responsetype: string;
        title: string;
        markdown: boolean;
        timeoutms: number;
        checkboxmsg: string;
        publictext: boolean;
        oklabel?: string;
        cancellabel?: string;
    };

    // userinput.UserInputResponse
    type UserInputResponse = {
        type: string;
        requestid: string;
        text?: string;
        confirm?: boolean;
        errormsg?: string;
        checkboxstat?: boolean;
    };

    type WSCommandType = {
        wscommand: string;
    } & ( SetBlockTermSizeWSCommand | BlockInputWSCommand | WSRpcCommand );

    // eventbus.WSEventType
    type WSEventType = {
        eventtype: string;
        oref?: string;
        data: any;
    };

    // wps.WSFileEventData
    type WSFileEventData = {
        zoneid: string;
        filename: string;
        fileop: string;
        data64: string;
        // File size immediately before this append (the chunk spans
        // [offset, offset + decoded(data64).length)). Only populated for
        // filestore-write-through-backed appends (handle_append_block_file);
        // absent means "no offset info available" — consumers should treat
        // that as "always new" (the pre-existing, always-write behavior).
        // Added so TermWrap can reconcile a chunk landing in the reconnect
        // window against what its own reconnect fetch already covered — see
        // SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.1 follow-up.
        offset?: number;
    };

    // webcmd.WSRpcCommand
    type WSRpcCommand = {
        wscommand: "rpc";
        message: RpcMessage;
    };

    // wconfig.WatcherUpdate
    type WatcherUpdate = {
        fullconfig: FullConfigType;
    };

    // wshrpc.WaveAIOptsType
    type WaveAIOptsType = {
        model: string;
        apitype?: string;
        apitoken: string;
        orgid?: string;
        apiversion?: string;
        baseurl?: string;
        proxyurl?: string;
        maxtokens?: number;
        maxchoices?: number;
        timeoutms?: number;
    };

    // wshrpc.WaveAIPacketType
    type WaveAIPacketType = {
        type: string;
        model?: string;
        created?: number;
        finish_reason?: string;
        usage?: WaveAIUsageType;
        index?: number;
        text?: string;
        error?: string;
    };

    // wshrpc.WaveAIPromptMessageType
    type WaveAIPromptMessageType = {
        role: string;
        content: string;
        name?: string;
    };

    // wshrpc.WaveAIStreamRequest
    type WaveAIStreamRequest = {
        clientid?: string;
        opts: WaveAIOptsType;
        prompt: WaveAIPromptMessageType[];
    };

    // wshrpc.WaveAIUsageType
    type WaveAIUsageType = {
        prompt_tokens?: number;
        completion_tokens?: number;
        total_tokens?: number;
    };

    // wps.WaveEvent
    type WaveEvent = {
        event: string;
        scopes?: string[];
        sender?: string;
        persist?: number;
        data?: any;
    };

    // filestore.WaveFile
    type WaveFile = {
        zoneid: string;
        name: string;
        opts: FileOpts;
        createdts: number;
        size: number;
        modts: number;
        meta: {[key: string]: any};
    };

    // wshrpc.WaveInfoData
    type WaveInfoData = {
        version: string;
        clientid: string;
        buildtime: string;
        configdir: string;
        datadir: string;
    };

    // vdom.WaveKeyboardEvent
    type WaveKeyboardEvent = {
        type: "keydown"|"keyup"|"keypress"|"unknown";
        key: string;
        code: string;
        repeat?: boolean;
        location?: number;
        shift?: boolean;
        control?: boolean;
        alt?: boolean;
        meta?: boolean;
        cmd?: boolean;
        option?: boolean;
    };

    // wshrpc.WaveNotificationOptions
    type WaveNotificationOptions = {
        title?: string;
        body?: string;
        silent?: boolean;
    };

    // waveobj.WaveObj
    type WaveObj = {
        otype: string;
        oid: string;
        version: number;
        meta: MetaType;
    };

    // waveobj.WaveObjUpdate
    type WaveObjUpdate = {
        updatetype: string;
        otype: string;
        oid: string;
        obj?: WaveObj;
    };

    // vdom.WavePointerData
    type WavePointerData = {
        button: number;
        buttons: number;
        clientx?: number;
        clienty?: number;
        pagex?: number;
        pagey?: number;
        screenx?: number;
        screeny?: number;
        movementx?: number;
        movementy?: number;
        shift?: boolean;
        control?: boolean;
        alt?: boolean;
        meta?: boolean;
        cmd?: boolean;
        option?: boolean;
    };

    // waveobj.Window
    type WaveWindow = WaveObj & {
        workspaceid: string;
        isnew?: boolean;
        pos: Point;
        winsize: WinSize;
        lastfocusts: number;
    };

    // wconfig.WebBookmark
    type WebBookmark = {
        url: string;
        title?: string;
        icon?: string;
        iconcolor?: string;
        iconurl?: string;
        "display:order"?: number;
    };

    // service.WebCallType
    type WebCallType = {
        service: string;
        method: string;
        uicontext?: UIContext;
        args: any[];
    };

    // service.WebReturnType
    type WebReturnType = {
        success?: boolean;
        error?: string;
        data?: any;
        updates?: WaveObjUpdate[];
    };

    // wshrpc.WebSelectorOpts
    type WebSelectorOpts = {
        all?: boolean;
        inner?: boolean;
    };

    // wconfig.WidgetConfigType
    type WidgetConfigType = {
        "display:order"?: number;
        "display:hidden"?: boolean;
        "display:pinned"?: boolean;
        icon?: string;
        color?: string;
        label?: string;
        description?: string;
        magnified?: boolean;
        blockdef: BlockDef;
    };

    // waveobj.WinSize
    type WinSize = {
        width: number;
        height: number;
    };

    // waveobj.Workspace
    type Workspace = WaveObj & {
        name?: string;
        tabids: string[];
        pinnedtabids: string[];
        activetabid: string;
    };

    // wshrpc.WorkspaceInfoData
    type WorkspaceInfoData = {
        windowid: string;
        workspacedata: Workspace;
    };

    // waveobj.WorkspaceListEntry
    type WorkspaceListEntry = {
        workspaceid: string;
        windowid: string;
    };

    // wshrpc.WshServerCommandMeta
    type WshServerCommandMeta = {
        commandtype: string;
    };

    // wshrpc.CommandSubprocessSpawnData
    type CommandSubprocessSpawnData = {
        blockid: string;
        tabid: string;
        cli_command: string;
        cli_args?: string[];
        working_dir?: string;
        env_vars?: {[key: string]: string};
        message: string;
    };

    // wshrpc.CommandAgentInputData
    type CommandAgentInputData = {
        blockid: string;
        message: string;
        message_id?: string;
    };

    // wshrpc.CommandAgentStopData
    type CommandAgentStopData = {
        blockid: string;
        force?: boolean;
    };

    // wshrpc.AgentConfigFile
    type AgentConfigFile = {
        path: string;
        content: string;
    };

    // wshrpc.CommandWriteAgentConfigData
    type CommandWriteAgentConfigData = {
        working_dir: string;
        files: AgentConfigFile[];
        // When true, treat working_dir as an auto-generated instance
        // path eligible for `<base>-N` collision resolution. When
        // false (user-specified path), write into the path as-is.
        auto_allocate?: boolean;
    };

    // wshrpc.CommandReadEditorFileData
    type CommandReadEditorFileData = {
        path: string;
    };

    // wshrpc.CommandReadEditorFileResult
    type CommandReadEditorFileResult = {
        content: string;
        read_only: boolean;
        // Detected text encoding (SPEC_EDITOR_FILE_ENCODINGS). Optional for
        // back-compat; absent ⇒ treat as UTF-8.
        encoding?: string;
        bom?: string;
        line_ending?: string;
        had_decode_errors?: boolean;
    };

    // wshrpc.CommandWriteEditorFileData
    type CommandWriteEditorFileData = {
        path: string;
        content: string;
        // Encoding to write back in; omit ⇒ UTF-8 (back-compat).
        encoding?: string;
        bom?: string;
        line_ending?: string;
    };

    // wshrpc.CommandResolveCliData
    type CommandResolveCliData = {
        provider_id: string;
        cli_command: string;
        npm_package: string;
        pinned_version: string;
        windows_install_command: string;
        unix_install_command: string;
        block_id?: string;
    };

    // wshrpc.ResolveCliResult
    type ResolveCliResult = {
        cli_path: string;
        version: string;
        source: string;
    };

    // wshrpc.CommandCheckCliAuthData
    type CommandCheckCliAuthData = {
        cli_path: string;
        auth_check_args: string[];
        auth_env?: {[key: string]: string};
    };

    // wshrpc.CheckCliAuthResult
    type CheckCliAuthResult = {
        authenticated: boolean;
        email?: string;
        auth_method?: string;
        raw_output: string;
    };

    // wshrpc.CommandRunCliLoginData
    type CommandRunCliLoginData = {
        cli_path: string;
        login_args: string[];
        auth_env?: {[key: string]: string};
    };

    // wshrpc.RunCliLoginResult
    type RunCliLoginResult = {
        auth_url?: string;
        raw_output: string;
    };

    // tool_store.ToolStatus
    type ToolStatus = "installed_system" | "installed_bundled" | "installed_managed" | "missing" | "unavailable";

    // tool_store.ToolStatusEntry
    type ToolStatusEntry = {
        id: string;
        display: string;
        description: string;
        tier: number;
        status: ToolStatus;
        version?: string;
        path?: string;
    };

    // wshrpc.GetToolStatusResult
    type GetToolStatusResult = {
        tools: ToolStatusEntry[];
    };

    // wshrpc.CommandInstallToolData
    type CommandInstallToolData = {
        tool_ids: string[];
    };

    // wshrpc.InstallFailure
    type InstallFailure = {
        id: string;
        error: string;
    };

    // wshrpc.InstallToolResult
    type InstallToolResult = {
        installed: string[];
        failed: InstallFailure[];
    };

    // wshrpc.CommandBlockfileLineCountData
    type CommandBlockfileLineCountData = {
        block_id: string;
        filename: string;
    };

    // wshrpc.BlockfileLineCountResult
    type BlockfileLineCountResult = {
        count: number;
    };

    // wshrpc.CommandBlockfileReadRangeData
    type CommandBlockfileReadRangeData = {
        block_id: string;
        filename: string;
        offset: number;
        limit: number;
    };

    // wshrpc.BlockfileReadRangeResult
    type BlockfileReadRangeResult = {
        lines: string[];
        total: number;
        // Receive-time stamps (unix ms) parallel to `lines`; 0 = unknown.
        // Absent when no output.tsidx sidecar exists (pre-upgrade history)
        // or the read skipped the output.idx fast path.
        stamps?: number[];
    };

    // wshrpc.CommandBlockfileReadStateData
    type CommandBlockfileReadStateData = {
        block_id: string;
        filename: string;
    };

    // wshrpc.BlockfileReadStateResult
    type BlockfileReadStateResult = {
        content: string | null;
    };

    // wshrpc.CommandBlockfileWriteStateData
    type CommandBlockfileWriteStateData = {
        block_id: string;
        filename: string;
        content: string;
    };

    // wshrpc.BlockfileWriteStateResult
    type BlockfileWriteStateResult = {
        bytes_written: number;
    };

    // wshrpc.CommandAgentSessionReadData — Option E (agent-anchored
    // session zones). Reads `output.state.json` from
    // `agent:<definition_id>:current`.
    type CommandAgentSessionReadData = {
        definition_id: string;
    };

    // wshrpc.AgentSessionReadResult — `content === null` (or undefined)
    // means no zone / snapshot exists for this definition (fresh
    // agent), NOT an error.
    type AgentSessionReadResult = {
        content?: string | null;
        modts?: number | null;
    };

    // wshrpc.CommandAgentSessionWriteStateData — writes
    // `output.state.json` into `agent:<definition_id>:current`
    // (creates the zone if missing).
    type CommandAgentSessionWriteStateData = {
        definition_id: string;
        content: string;
    };

    // wshrpc.AgentSessionWriteStateResult
    type AgentSessionWriteStateResult = {
        bytes_written: number;
    };

    // wshrpc.CommandAgentSessionAppendOutputData — appends a single
    // NDJSON line to `output` in `agent:<definition_id>:current`.
    type CommandAgentSessionAppendOutputData = {
        definition_id: string;
        line: string;
    };

    // wshrpc.AgentSessionAppendOutputResult
    type AgentSessionAppendOutputResult = {
        bytes_written: number;
    };

    // wshrpc.CommandAgentSessionArchiveData — snapshots
    // `agent:<defId>:current` into `agent:<defId>:archive:<now_ms>`
    // then clears the current zone.
    type CommandAgentSessionArchiveData = {
        definition_id: string;
    };

    // wshrpc.AgentSessionArchiveResult — empty `archive_zoneid` when
    // nothing was archived (current zone was empty).
    type AgentSessionArchiveResult = {
        archive_zoneid: string;
        archived_at_ms: number;
    };

    // wshrpc.CommandAgentSessionListArchivesData
    type CommandAgentSessionListArchivesData = {
        definition_id: string;
        limit?: number;
    };

    // wshrpc.AgentArchiveRow — one row of the agent's archive list.
    type AgentArchiveRow = {
        archive_zoneid: string;
        archived_at_ms: number;
        preview: string;
        node_count: number;
    };

    // wshrpc.NativeMemoryFileMeta — one *.md file in the agent's native memory folder.
    type NativeMemoryFileMeta = {
        filename: string;
        is_index: boolean;
        metadata_type: string | null;
        size_bytes: number; // u64 on the wire; safe for files up to 2^53 bytes (~8 PB)
        modified_at: number;
    };

    // wshrpc.NativeMemoryListResult
    type NativeMemoryListResult = {
        files: NativeMemoryFileMeta[];
    };

    // wshrpc.NativeMemoryReadFileResult
    type NativeMemoryReadFileResult = {
        content: string;
    };

    // wshrpc.CommandActivitySummaryData
    type CommandActivitySummaryData = {
        block_id: string;
        word_target?: number;
        // Caller's monotonic turn counter for this block. The ambient-call
        // gateway uses it to cancel a stale in-flight request for the same
        // block and reject a request that arrives out of order.
        generation: number;
    };

    // wshrpc.ActivitySummaryResult
    type ActivitySummaryResult = {
        summary: string;
        // Absent when the request was rejected as stale-on-arrival or the
        // underlying call failed/was cancelled.
        tokens?: TokenCounts;
    };

    // wshrpc.CommandNextPromptSuggestionData
    type CommandNextPromptSuggestionData = {
        block_id: string;
        // Same generation contract as CommandActivitySummaryData.
        generation: number;
    };

    // wshrpc.NextPromptSuggestionResult
    type NextPromptSuggestionResult = {
        suggestion: string;
        // Absent under the same conditions as ActivitySummaryResult.tokens.
        tokens?: TokenCounts;
    };

    // wshrpc.CommandSessionArchiveData
    type CommandSessionArchiveData = {
        block_id: string;
    };

    // wshrpc.SessionArchiveResult
    type SessionArchiveResult = {
        block_id: string;
        archived_bytes: number;
        archived_at: number;
    };

    // wshrpc.CommandSessionRestoreData
    type CommandSessionRestoreData = {
        block_id: string;
    };

    // wshrpc.SessionRestoreResult
    type SessionRestoreResult = {
        block_id: string;
        restored_bytes: number;
    };

    // wshrpc.CommandSessionExportData
    type CommandSessionExportData = {
        block_id: string;
    };

    // wshrpc.SessionExportResult
    type SessionExportResult = {
        /** base64-encoded JSONL content (raw output file bytes) */
        content: string;
        line_count: number;
        byte_count: number;
    };

    // CommandImportAgentDefinitionsData
    type AgentSkillImport = {
        name: string;
        trigger: string;
        skill_type: string;
        description: string;
        content: string;
    };

    type AgentDefinitionImport = {
        id: string;
        name: string;
        icon: string;
        description: string;
        provider: string;
        shell: string;
        working_directory: string;
        agent_bus_id: string;
        agent_type: string;
        environment: string;
        restart_on_crash: boolean;
        content: Record<string, string>;
        skills: AgentSkillImport[];
    };

    type CommandImportAgentDefinitionsData = {
        agents: AgentDefinitionImport[];
    };

    type ImportAgentDefinitionsResult = {
        imported: string[];
        skipped: string[];
        failed: string[];
    };

    // ExportAgentDefinitionsResult
    type AgentSkillExport = {
        name: string;
        trigger: string;
        skill_type: string;
        description: string;
        content: string;
    };

    type AgentDefinitionExport = {
        id: string;
        name: string;
        icon: string;
        description: string;
        provider: string;
        shell: string;
        working_directory: string;
        agent_bus_id: string;
        agent_type: string;
        environment: string;
        restart_on_crash: boolean;
        content: Record<string, string>;
        skills: AgentSkillExport[];
    };

    type ExportAgentDefinitionsResult = {
        version: number;
        exported_at: string;
        source: string;
        agents: AgentDefinitionExport[];
    };

}

export {}
