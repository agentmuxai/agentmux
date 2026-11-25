// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's side of the links layer (SPEC_LINKS_ON_EVERYTHING_2026_11_20.md §7, build 2): what a card tells the
// resolver about its text, and the tasks it publishes so a number in any text can be checked against the boards.

import { publishTasks } from "@/app/links/task-index-store";
import { publishHeldTasks } from "@/app/store/held-tasks";
import type { SurfaceContext } from "@/app/links/link-context";
import { boardSlugOf, defaultSlugOf } from "./kanban-boards";
import type { KanbanBoard, KanbanTask } from "./kanban-types";

/** The repository and board a task's text belongs to. The text is written by agents and by the operator; no file
 *  is linked from it yet, so the author is the app until the build that links files names one. */
export function surfaceOf(t: KanbanTask): SurfaceContext {
    return { repo: t.card.repo || undefined, board: t.card.board || undefined };
}

/** Every board's tasks as the numbers and titles the resolver checks a reference against. */
export function publishBoardTasks(raw: KanbanBoard): void {
    // The task each agent pane holds, for an agent tab's hover card (store/held-tasks.ts).
    publishHeldTasks(raw.tasks);
    const fallback = defaultSlugOf(raw);
    publishTasks(
        raw.tasks.map((t) => ({ board: boardSlugOf(t, fallback), number: t.card.num, title: t.title, state: t.state })),
        fallback
    );
}
