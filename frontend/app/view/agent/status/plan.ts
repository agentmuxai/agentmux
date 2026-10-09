// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent's own plan, from its todo list (`TodoWrite`): which step it is on.
 * Each item carries an `activeForm` ("Writing the spec") the model writes for
 * exactly this, the line Claude Code's own spinner shows. Read from the tool
 * call's input, so it is the model's words, not ours.
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.2.
 */

export interface PlanState {
    /** The step in progress, in the model's progressive form. */
    activeForm: string;
    /** 1-based position of that step, and the list's length. */
    index: number;
    total: number;
}

/** The plan a `TodoWrite` call sets, or null when no step is in progress
 *  (nothing started yet, or every step done). */
export function planFromTodoWrite(params: Record<string, unknown> | undefined): PlanState | null {
    const todos = params?.todos;
    if (!Array.isArray(todos) || todos.length === 0) return null;
    const i = todos.findIndex((t) => (t as { status?: unknown } | null)?.status === "in_progress");
    if (i < 0) return null;
    const item = todos[i] as { activeForm?: unknown; content?: unknown };
    const text =
        typeof item.activeForm === "string" && item.activeForm.trim()
            ? item.activeForm.trim()
            : typeof item.content === "string"
              ? item.content.trim()
              : "";
    if (!text) return null;
    return { activeForm: text.replace(/\s+/g, " "), index: i + 1, total: todos.length };
}

/** "Writing the spec (3/7)". */
export function planLine(plan: PlanState): string {
    return plan.total > 1 ? `${plan.activeForm} (${plan.index}/${plan.total})` : plan.activeForm;
}
