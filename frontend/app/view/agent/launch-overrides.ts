// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a caller of `launchAgentDefinition` can set for one launch: the
 * instance name, the runtime, and a continuation or fork of an earlier
 * instance. The agent picker (a new agent, a reattach, a relaunch) and
 * quick fork build these.
 */
export interface LaunchOverrides {
    /** Instance name — written into AGENTMUX_AGENT_ID and used to
     *  derive the working directory. */
    instanceName: string;
    /** "host" runs directly on the OS. "container" runs inside
     *  Docker/Podman. */
    agentType: "host" | "container";
    /** "local" pairs with "host"; "docker" pairs with "container". */
    environment: "local" | "docker";
    /** Only set when agentType === "container". */
    containerImage?: string;
    /** Account to bind to the new instance; '' binds none. Issue #1624
     *  PR-C Part B — was `identityId` (a bundle id). */
    accountId: string;
    /** The launch's own record of a bundle (`db_agents.memory_id`): the
     *  first of the agent's Bundles list, else a continued row's, else ''.
     *  Read back by the Continue and Recent lists; launch doesn't use it.
     *  The list itself is saved to the agent before the launch
     *  (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6). */
    bundleId: string;
    /** v8 — when set, this launch is a continuation of a prior named
     *  agent. The id is recorded as `parent_instance_id` on the new
     *  row so the lineage is queryable. */
    continueOfInstanceId?: string;
    /** v8 — when set (paired with `continueOfInstanceId`), the
     *  launch flow uses this exact path instead of calling
     *  `allocate_agent_workdir`. Reuses the prior agent's files,
     *  configs, and conversation context. */
    workDirOverride?: string;
    /** Two-tier picker (2026-05-24) — CLI-emitted session id of the
     *  prior instance. When non-empty, written to the new block's
     *  `agent:sessionid` meta so the spawned subprocess can pass
     *  `--resume <sid>` on its FIRST turn. Without this the
     *  reattached pane starts a fresh CLI session and re-injects the
     *  startup context. */
    continueSessionId?: string;
    /** In-pane tabs, Phase 4 (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md
     *  §4.1) — when set alongside `continueSessionId`, appends `--fork-session`
     *  to the spawned CLI's args so it inherits history up to the resume
     *  point and then diverges with a NEW session id, instead of continuing
     *  the same session (which `continueSessionId` alone would do). Only
     *  applied for the Claude provider specifically — the only one
     *  `--fork-session` was ever validated against
     *  (SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15 §6.4). Every other
     *  provider naturally falls back to "fork = fresh definition, fresh
     *  start" by simply not getting the flag. */
    forkSession?: boolean;
    /** Initial model choice (a value from the effective provider's own
     *  `models` list) — seeds the new block's `agent:runtime` meta at
     *  launch, the same key `AgentRuntimeDropup`'s in-session model
     *  picker writes to via `applyRuntimeChange`. When omitted,
     *  `launchAgentDefinition` falls back to the provider's own
     *  `default: true` model, or `DEFAULT_RUNTIME_CONFIG.model` if the
     *  provider declares none. Callers that let the user pick a model at
     *  creation time (e.g. AgentCreateForm) set this so the
     *  first launch doesn't silently start on an unrelated provider's
     *  default model. */
    model?: string;
    /** Settings carried over from another pane (a fork of a running agent), so the
     *  fork starts as the source was running instead of on the defaults. Each
     *  field is optional; `model` above (an explicit launch choice) wins over it. */
    carryOverRuntime?: Partial<import("./types").AgentRuntimeConfig>;
}
