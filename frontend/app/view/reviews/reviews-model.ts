// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Reviews pane's list, as groups (docs/specs/SPEC_REVIEWS_PANE_2026_11_15.md §5, §8): the rows srv answers with
 * (`GET /api/v1/reviews`) grouped by the Kanban task they belong to, with the pull requests that wait on a person
 * first. Everything here is pure; it holds no state and reads no clock but the one it is given.
 */

import type { ReviewRow } from "@/types/rpc/ReviewRow";
import type { WaitsOn } from "@/types/rpc/WaitsOn";

/** The pane's own filters: what the person typed, and which waits-on to show. */
export interface ReviewsFilter {
    text: string;
    /** Only the rows that wait on this; null for all. */
    waitsOn: WaitsOn | null;
}

export const NO_FILTER: ReviewsFilter = { text: "", waitsOn: null };

/** A group of rows under one heading. */
export interface ReviewsGroup {
    /** Stable across refreshes: `person`, `task:<num>` or `none`. */
    key: string;
    kind: "person" | "task" | "none";
    /** A task group's heading, its number and title; empty for the two fixed groups, whose words are the view's (reviews-text.ts). */
    title: string;
    /** The task's own state, for a task group. */
    state?: string;
    rows: ReviewRow[];
}

/**
 * A row's identity: its repository and number, or, with no repository, the repository id and number; with its task's
 * ref after it when a task holds it, so two tasks that name one pull request are two rows, each selected on its own.
 */
export function rowKey(row: ReviewRow): string {
    const pr = `${row.repo ?? `id:${row.repo_id ?? 0}`}#${row.number}`;
    return row.task ? `${pr}@${row.task.ref}` : pr;
}

/** Words that name a row, for the search box: the number, the task, the agent, the repository, the branch. */
function haystack(row: ReviewRow): string {
    return [
        `#${row.number}`,
        row.task ? `${row.task.ref} ${row.task.title}` : "",
        row.agent ?? "",
        row.repo ?? "",
        row.branch,
        row.waits_on,
        row.why.replace(/_/g, " "),
    ]
        .join(" ")
        .toLowerCase();
}

/** The rows `filter` keeps: every word typed must be in a row's words. */
export function filterRows(rows: readonly ReviewRow[], filter: ReviewsFilter): ReviewRow[] {
    const words = filter.text.toLowerCase().split(/\s+/).filter(Boolean);
    return rows.filter((row) => {
        if (filter.waitsOn && row.waits_on !== filter.waitsOn) return false;
        if (words.length === 0) return true;
        const text = haystack(row);
        return words.every((w) => text.includes(w));
    });
}

/**
 * The rows as groups: "Needs a person" first (oldest change first, since the one that has waited longest is the one
 * to do next), then a group for each task, the one that changed most recently first, then the rows with no task.
 * A row that waits on a person is in "Needs a person" only, so it is shown once.
 */
export function groupRows(rows: readonly ReviewRow[]): ReviewsGroup[] {
    const person = rows.filter((r) => r.waits_on === "person").sort((a, b) => a.updated_at_ms - b.updated_at_ms);
    const rest = rows.filter((r) => r.waits_on !== "person");
    const byTask = new Map<string, ReviewsGroup>();
    const none: ReviewRow[] = [];
    for (const row of rest) {
        if (!row.task) {
            none.push(row);
            continue;
        }
        const group = byTask.get(row.task.ref) ?? {
            key: `task:${row.task.ref}`,
            kind: "task" as const,
            title: `${row.task.ref} ${row.task.title}`.trim(),
            state: row.task.state,
            rows: [],
        };
        group.rows.push(row);
        byTask.set(row.task.ref, group);
    }
    const newest = (g: ReviewsGroup) => Math.max(...g.rows.map((r) => r.updated_at_ms));
    const groups: ReviewsGroup[] = [];
    if (person.length > 0) groups.push({ key: "person", kind: "person", title: "", rows: person });
    groups.push(...[...byTask.values()].sort((a, b) => newest(b) - newest(a)));
    if (none.length > 0) groups.push({ key: "none", kind: "none", title: "", rows: none });
    return groups;
}

/**
 * The board-qualified ref a `reviews:task` value or a `?task=` names: `#12` for a bare number or `#12` (the default
 * board's task), `work#12` as it is. Task numbers are per board, so a bare number never means another board's task.
 */
export function taskRefOf(value: string): string {
    const v = value.trim().toLowerCase();
    return /^#?\d+$/.test(v) ? `#${v.replace(/^#/, "")}` : v;
}

/** Whether `row` belongs to the task `value` names. */
export function inTaskRef(row: ReviewRow, value: string): boolean {
    return !!row.task && row.task.ref.toLowerCase() === taskRefOf(value);
}

/** The id the Kanban opens a task by: its ref without the leading `#` (`12`, or `work#12` off the default board). */
export function kanbanIdOf(ref: string): string {
    return ref.replace(/^#/, "");
}
