// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's data as srv's `kanban.*` commands send it
// (crates/srv/src/server/app_api/kanban.rs, docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md §7).

/** A task's state in the queue (`work_state` in crates/srv/src/backend/storage/work_queue.rs). */
export type TaskState = "open" | "held" | "claimed" | "blocked" | "review" | "done" | "failed" | "cancelled";

/** An agent's state as srv decides it (`AgentState`), or `gone` when it isn't registered. */
export type KanbanAgentState = "working" | "waiting" | "idle" | "stopped" | "error" | "limited" | "signed_out" | "gone";

/** Why a task's holder can't work (`Unavailable` in crates/srv/src/backend/agent_state.rs): out of quota, or
 *  signed out. Absent from an older srv, and while the holder can work. */
export interface KanbanUnavailable {
    state: "limited" | "signed_out";
    title: string;
    /** When the limit lifts (ms), when the provider said. */
    resets_at_ms?: number | null;
    /** Since when it has been in this state (ms). */
    since_ms: number;
}

export interface ChecklistItem {
    text: string;
    status: string;
}

/** A task another starts after, or one that starts after it (`AfterTask` in
 *  crates/srv/src/backend/storage/work_deps.rs). */
export interface KanbanAfter {
    id: string;
    num: number;
    title: string;
    state: TaskState;
    /** Its kind (`spec`, `research`, `build`, …; absent from an older srv). */
    kind?: string;
    /** The path of the document it wrote, empty when none. */
    doc?: string;
}

/** What srv learned about a task's PR (`WorkPr` in crates/srv/src/backend/storage/work_prs.rs). */
export interface KanbanPr {
    /** When it merged (ms), once srv has seen it. */
    merged_at?: number | null;
    /** The release that contains the merge (its tag), once one does. */
    released_in?: string;
    released_at?: number | null;
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
    /** The tasks that start after it (absent from an older srv). */
    followers?: KanbanAfter[];
    /** Its PR's merge and release, once srv has seen them. */
    pr?: KanbanPr | null;
    /** The path of the spec or report it wrote (`links.doc`), empty when none. */
    doc?: string;
}

export interface KanbanAgent {
    name: string;
    uid?: string | null;
    block_id: string;
    state: KanbanAgentState;
    /** The colour its definition keeps (`#rrggbb`), absent from an older srv. */
    color?: string | null;
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
    /** Why that agent can't work, when it can't. */
    holder_unavailable?: KanbanUnavailable | null;
    /** Set on a Done task a leader accepted on the operator's word, in the last 7 days (the operator can reopen it). */
    accepted_by?: KanbanAcceptedBy | null;
}

/** A leader's acceptance, on the operator's word. */
export interface KanbanAcceptedBy {
    name: string;
    /** The history line's body: the operator's quoted words. */
    said: string;
    at: number;
}

export interface KanbanBoard {
    tasks: KanbanTask[];
    agents: KanbanAgent[];
    /** The colours of agents tasks name that aren't running here, by uid (absent from an older srv). */
    agent_colors?: Record<string, string>;
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
    action:
        | "create"
        | "edit"
        | "move"
        | "start"
        | "answer"
        | "accept"
        | "send_back"
        | "reopen"
        | "cancel"
        | "retry"
        | "track"
        | "release"
        | "reassign";
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
    /** release, reassign: the operator read the warning (the task has a branch or PR) and wants it anyway. */
    confirm?: boolean;
    /** create: "spec" or "research" files that task and a build that starts after it. */
    first?: "spec" | "research";
    /** cancel, a spec or research task: what happens to the tasks waiting on it. */
    followers?: "cancel" | "hold";
}

export interface KanbanActResult {
    ok: boolean;
    /** release, reassign: nothing changed; the words to ask the operator, who answers by acting again with `confirm`. */
    confirm?: string;
    id?: string;
    num?: number;
    attempt?: number;
}
