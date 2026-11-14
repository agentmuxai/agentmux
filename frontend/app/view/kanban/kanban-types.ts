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
    /** Its number on its board. */
    num: number;
    /** The board it is on: its id and its slug (absent from an older srv, which had one board). */
    board_id?: string;
    board?: string;
    /** How a message names it: `#12` on the default board, `work#12` elsewhere (absent from an older srv). */
    task_ref?: string;
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
    /** The files it carries and the UI-change flag (absent from an older srv). */
    media?: KanbanMediaSummary;
}

/** A thumbnail on the card (`MediaPreview` in crates/srv/src/backend/storage/work_media.rs). */
export interface KanbanMediaPreview {
    sha256: string;
    kind: "image" | "recording";
    mime: string;
    /** Where the stored file is, for the Editor and the thumbnail; empty if srv could not say. */
    path: string;
    /** The small copy of an image, empty when there is none (an SVG, a recording). */
    thumb: string;
}

/** What the card shows of a task's media (`MediaSummary`). */
export interface KanbanMediaSummary {
    count: number;
    first: KanbanMediaPreview[];
    /** The task changes the UI, so handing it in for review needs media. */
    ui_change: boolean;
    /** Why it was handed in without media, when it was. */
    no_media_reason: string;
}

/** One attached file, as `kanban.task` lists it (`MediaItem`). */
export interface KanbanMediaItem {
    id: number;
    work_id: string;
    sha256: string;
    name: string;
    kind: "image" | "recording";
    mime: string;
    size: number;
    caption: string;
    attached_by: string;
    attempt: number;
    at: number;
    /** When it was removed; it then has no path. */
    removed_at?: number | null;
    /** Who removed it: "operator", or an agent's name. */
    removed_by?: string;
    path: string;
    thumb: string;
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
    /** Set on a Done task an agent accepted, in the last 7 days (the operator can reopen it). */
    accepted_by?: KanbanAcceptedBy | null;
}

/** An agent's acceptance. */
export interface KanbanAcceptedBy {
    name: string;
    /** The history line's body: the agent's note, and that it was its own work when it was. */
    said: string;
    at: number;
}

/** One board in the picker (`board_json` in crates/srv/src/server/work_boards.rs). */
export interface KanbanBoardInfo {
    id: string;
    /** Lowercase letters, digits and hyphen: what a ref says (`work#12`). */
    slug: string;
    name: string;
    /** `local` (this machine); `lan` and `cloud` come with sync. */
    tier: string;
    readable: boolean;
    /** The agents that see it; empty means all. */
    agents: string[];
    default: boolean;
    archived: boolean;
    /** Tasks not finished, and all tasks. */
    open: number;
    total: number;
    /** While a delete waits out its countdown: when it happens (ms). */
    pending_delete_at?: number | null;
}

export interface KanbanBoard {
    tasks: KanbanTask[];
    agents: KanbanAgent[];
    /** Every board there is (absent from an older srv, which had one). */
    boards?: KanbanBoardInfo[];
    /** The board the tasks are of: its slug, or `all`. */
    board?: string;
    /** The slug of the default board. */
    default_board?: string | null;
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
    /** Every file the task carries, oldest first, the removed ones marked (absent from an older srv). */
    media?: KanbanMediaItem[];
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
        | "reassign"
        | "media_remove"
        | "ui_change"
        | "board_create"
        | "board_edit"
        | "board_archive"
        | "board_unarchive"
        | "board_default"
        | "board_delete"
        | "board_cancel_delete";
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
    /** media_remove: the file to take off the task. */
    media_id?: number;
    /** ui_change: whether the task changes the UI. */
    ui_change?: boolean;
    /** create: the board to file on; move: the board to move the task to; board_*: the board acted on (name or slug). */
    board?: string;
    /** move to a board: remove the dependencies the task has, which stay inside one board. */
    drop_deps?: boolean;
    /** board_create, board_edit: the display name; the slug stays unless `slug` is given. */
    name?: string;
    slug?: string;
    /** board_edit: the agents that see the board; `[]` means all. */
    agents?: string[];
}

export interface KanbanActResult {
    ok: boolean;
    /** release, reassign: nothing changed; the words to ask the operator, who answers by acting again with `confirm`. */
    confirm?: string;
    id?: string;
    num?: number;
    attempt?: number;
}
