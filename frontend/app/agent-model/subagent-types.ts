// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17: the backend now models
// two levels — `AgentDispatch` (one per Agent-tool-or-Workflow-tool call)
// containing N `SubAgent` members. The TS interface here keeps the
// `ActiveSubagent` name (unlike the Rust `SubagentInfo`→`SubAgent` rename)
// to limit churn across this file and swarm-view.tsx — it maps 1:1 to the
// backend's `SubAgent` either way.
export interface ActiveSubagent {
    agent_id: string;
    slug: string;
    parent_agent: string;
    parent_block_id: string;
    session_id: string;
    /** `"abandoned"` (SubAgentStatus::Abandoned, Rust) — the parent block's
     *  turn ended without a Result line ever appearing for this subagent
     *  (crashed, killed, or interrupted by an app/srv restart). Distinct
     *  from `"completed"`: it didn't finish, it was cut off. Set by the
     *  backend's `reconcile_stale_subagents`, currently only on pane
     *  reopen/backfill. */
    status: "active" | "completed" | "abandoned";
    /** Unix ms when this subagent was first observed — set once, immutable.
     *  Distinct from `last_event_at`, which advances on every journal read. */
    spawned_at: number;
    last_event_at: number;
    event_count: number;
    model: string | null;
    /** Always present now (was `workflow_id: string | null`) — every
     *  subagent has a real dispatch container. A Workflow-tool member
     *  carries the run's own id (`"wf_<id>"`); a solo Task-tool call gets a
     *  synthesized `"solo:<agent_id>"` (Rust's `solo_dispatch_id`). Use
     *  `dispatch_id.startsWith("solo:")` to tell them apart client-side. */
    dispatch_id: string;
    // Concise Haiku-generated name (SubAgent.display_name, Rust). Null
    // until a client expands this subagent's row for the first time — see
    // `subagent.GenerateName` / the `subagent:named` event below.
    display_name: string | null;
    /** The Agent call that spawned this subagent (SubAgent.tool_use_id, from
     *  its meta.json). Joins it to the background tasks it launched
     *  (`BackgroundTaskView.owner_tool_use_id`). Null until the sidecar is
     *  readable; absent from older srv builds. */
    tool_use_id?: string | null;
}
