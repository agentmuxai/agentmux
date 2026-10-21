// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's data as srv's `kanban.*` commands send it
// (crates/srv/src/server/app_api/kanban.rs, docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md §7).

/** A task's state in the queue (`work_state` in crates/srv/src/backend/storage/work_queue.rs). */
export type TaskState = "open" | "held" | "claimed" | "blocked" | "review" | "done" | "failed" | "cancelled";

/** An agent's state as srv decides it (`AgentState`), or `gone` when it isn't registered. */
export type KanbanAgentState = "working" | "waiting" | "idle" | "stopped" | "error" | "gone";

export interface ChecklistItem {
    text: string;
    status: string;
}

/** A task another starts after (`AfterTask` in crates/srv/src/backend/storage/work_deps.rs). */
export interface KanbanAfter {
    id: string;
    num: number;
    title: string;
    state: TaskState;
}

export interface KanbanCardFields {
    num: number;
    repo: string;
    branch: string;
    pr_url: string;
    rank?: number | null;
    review_required: boolean;
    checklist: ChecklistItem[];
    question: string;
    note: string;
    requested_by: string;
    /** `operator`, `agent`, `other` or `unknown`. */
    requested_by_kind: string;
    request_text: string;
    filed_by: string;
    last_reminded_at?: number | null;
    updated_at: number;
    /** The tasks it starts after, finished or not (absent from an older srv). */
    after?: KanbanAfter[];
}

export interface KanbanAgent {
    name: string;
    uid?: string | null;
    block_id: string;
    state: KanbanAgentState;
}

export interface KanbanTask {
    id: string;
    title: string;
    payload: string;
    kind: string;
    target_agent: string;
    target_agent_uid: string;
    priority: number;
    state: TaskState;
    claimed_by: string;
    claimed_by_uid: string;
    claim_expires?: number | null;
    attempts: number;
    max_attempts: number;
    created_by: string;
    created_at: number;
    updated_at: number;
    result: string;
    card: KanbanCardFields;
    /** The agent holding it, when one does and it is registered here. */
    holder: KanbanAgent | null;
}

export interface KanbanBoard {
    tasks: KanbanTask[];
    agents: KanbanAgent[];
    /** The newest change, to tell whether anything moved. */
    stamp: number;
    now: number;
}

export interface KanbanEvent {
    id: number;
    work_id: string;
    at: number;
    actor: string;
    actor_kind: string;
    kind: string;
    from_state: string;
    to_state: string;
    body: string;
}

export interface KanbanTaskDetail {
    task: KanbanTask;
    events: KanbanEvent[];
}

/** `kanban.act`'s request: one of the operator's moves. */
export interface KanbanActRequest {
    action: "create" | "edit" | "move" | "start" | "answer" | "accept" | "send_back" | "cancel" | "retry" | "track";
    id?: string;
    title?: string;
    payload?: string;
    agent?: string;
    hold?: boolean;
    review?: boolean;
    repo?: string;
    kind?: string;
    priority?: number;
    rank?: number;
    text?: string;
    block_id?: string;
    /** create, edit: the tasks it starts after, by id or number; edit replaces the list. */
    after?: string[];
}

export interface KanbanActResult {
    ok: boolean;
    id?: string;
    num?: number;
    attempt?: number;
}
