// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Where each task sits on the Kanban, and in what order (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md
// §6.3, §6.5; the stages: SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §4). Pure: the section
// and its tests share it.

import type { AccountAvailability } from "@/types/rpc/AccountAvailability";
import { resetLabel } from "@/util/reset-label";

import type { KanbanAfter, KanbanAgent, KanbanTask } from "./kanban-types";

export type ColumnId = "backlog" | "research" | "specd" | "progress" | "needs" | "review" | "merged" | "released";

export const COLUMNS: { id: ColumnId; label: string }[] = [
    { id: "backlog", label: "Backlog" },
    { id: "research", label: "In Research" },
    { id: "specd", label: "Spec'd" },
    { id: "progress", label: "In progress" },
    { id: "needs", label: "Needs you" },
    { id: "review", label: "Review" },
    { id: "merged", label: "Merged" },
    { id: "released", label: "Released" },
];

/** The Kanban section's width at or under which the board is one list (§8 of the stages spec); `kanban.scss`'s
 *  `$kanban-narrow` is the same number. */
export const KANBAN_NARROW_PX = 760;

/** The kinds of task that write a document for the operator to approve: a build that starts after one
 *  of them is Spec'd once it is accepted. Matched case-insensitively. */
const RESEARCH_KINDS = new Set(["spec", "research"]);

export function isResearchKind(kind: string | undefined): boolean {
    return !!kind && RESEARCH_KINDS.has(kind.trim().toLowerCase());
}

/** The columns a finished task sits in. */
export function isFinishedColumn(c: ColumnId): boolean {
    return c === "merged" || c === "released";
}

/** Approved and not started: a waiting task whose every predecessor is finished, at least one of them a
 *  spec or research task that was accepted (done), not cancelled. */
export function isSpecd(t: KanbanTask): boolean {
    const after = t.card.after ?? [];
    const research = after.filter((a) => isResearchKind(a.kind));
    return (
        research.length > 0 &&
        after.every((a) => a.state === "done" || a.state === "cancelled") &&
        research.some((a) => a.state === "done")
    );
}

/** A word for what a task is: "spec", "research" or "build" (any other kind, or none). */
export function stageWord(t: KanbanTask): string {
    return isResearchKind(t.kind) ? t.kind.trim().toLowerCase() : "build";
}

/** "out of quota until Sun 7:00 PM", or "out of quota" when the provider named no time. */
export function quotaReason(resetsAtMs: number | null | undefined): string {
    return resetsAtMs ? `out of quota until ${resetLabel(resetsAtMs)}` : "out of quota";
}

/** Why a task needs the operator, or null when it doesn't. The first line of its card in Needs you. */
export function needsReason(t: KanbanTask): string | null {
    switch (t.state) {
        case "blocked":
            return t.card.question ? `asks: ${t.card.question}` : "asks you something";
        case "failed":
            return `failed after ${t.attempts} attempt${t.attempts === 1 ? "" : "s"}`;
        case "claimed": {
            const h = t.holder;
            if (!h || h.state === "gone") return "agent closed";
            if (h.state === "waiting") return "waiting on you";
            if (h.state === "limited") return quotaReason(t.holder_unavailable?.resets_at_ms);
            if (h.state === "signed_out") return "needs sign-in";
            if (h.state === "error") return "agent errored";
            if (h.state === "stopped") return "agent stopped";
            return null;
        }
        default:
            return null;
    }
}

/** The column a task belongs in: its stored state, its kind and what it starts after, its PR, and its
 *  holder's live state. Review is the PR's review (muxreview), not the operator's: a held task with a PR
 *  link sits there until the PR merges. A cancelled task sits under Merged, folded away. */
export function columnOf(t: KanbanTask): ColumnId {
    switch (t.state) {
        case "open":
        case "held":
            return isSpecd(t) ? "specd" : "backlog";
        case "review":
            return "review";
        case "done":
            return t.card.pr?.released_in ? "released" : "merged";
        case "cancelled":
            return "merged";
        case "blocked":
        case "failed":
            return "needs";
        case "claimed":
            if (needsReason(t)) return "needs";
            if (t.card.pr?.merged_at) return "merged";
            if (t.card.pr_url) return "review";
            return isResearchKind(t.kind) ? "research" : "progress";
        default:
            return "backlog";
    }
}

/** An account episode as far as the count is concerned: the agents it covers, by definition id and by pane. */
export type CoveringEpisode = Pick<AccountAvailability, "agent_ids" | "block_ids">;

/** Whether a task is in Needs you only because its holder is out of quota or signed out. */
export function isUnavailableHolder(t: KanbanTask): boolean {
    return t.state === "claimed" && (t.holder?.state === "limited" || t.holder?.state === "signed_out");
}

/** Whether an open account episode already covers this task's holder: the episode is one item for all its
 *  agents (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.2), so its tasks aren't counted again. */
export function coveredByEpisode(t: KanbanTask, episodes: readonly CoveringEpisode[]): boolean {
    const block = t.holder?.block_id;
    const uids = [t.claimed_by_uid, t.holder?.uid].filter((u): u is string => !!u);
    return episodes.some((e) => (!!block && e.block_ids.includes(block)) || uids.some((u) => e.agent_ids.includes(u)));
}

/** How many tasks need the operator: the Swarm's title count on the Kanban and the widget-bar badge's. A task whose
 *  holder is out of quota or signed out still sits in Needs you with its reason, but is counted only when no
 *  open account episode covers that holder (e.g. no run account could be resolved); the episodes are counted
 *  once each by the caller. */
export function needsCountOf(tasks: KanbanTask[], episodes: readonly CoveringEpisode[] = []): number {
    return tasks.filter((t) => columnOf(t) === "needs" && !(isUnavailableHolder(t) && coveredByEpisode(t, episodes))).length;
}

/** A waiting task's place in its backlog: the operator's rank, else when it was filed (srv's claim order). */
export function rankOf(t: KanbanTask): number {
    return t.card.rank ?? t.created_at;
}

/** srv's claim order within one agent's backlog: priority, then rank. */
export function backlogOrder(a: KanbanTask, b: KanbanTask): number {
    return b.priority - a.priority || rankOf(a) - rankOf(b);
}

/** A rank that puts a task between two others (either end may be missing). */
export function rankBetween(before: number | undefined, after: number | undefined): number {
    if (before === undefined && after === undefined) return Date.now();
    if (before === undefined) return (after as number) - 1000;
    if (after === undefined) return before + 1000;
    return (before + after) / 2;
}

export interface BacklogGroup {
    /** The agent's name, or "" for Anyone. */
    key: string;
    label: string;
    agent: KanbanAgent | undefined;
    tasks: KanbanTask[];
}

/** A waiting column (Backlog, or Spec'd) grouped by who each task waits for: one group per agent,
 *  alphabetically, then Anyone. */
export function backlogGroups(
    tasks: KanbanTask[],
    agents: KanbanAgent[],
    column: ColumnId = "backlog"
): BacklogGroup[] {
    const groups = new Map<string, KanbanTask[]>();
    for (const t of tasks) {
        if (columnOf(t) !== column) continue;
        const key = t.target_agent || "";
        groups.set(key, [...(groups.get(key) ?? []), t]);
    }
    const out: BacklogGroup[] = [...groups.entries()].map(([key, list]) => ({
        key,
        label: key || "Anyone",
        agent: key ? agents.find((a) => a.name === key || (!!a.uid && a.uid === list[0]?.target_agent_uid)) : undefined,
        tasks: [...list].sort(backlogOrder),
    }));
    return out.sort((a, b) => (a.key === "" ? 1 : b.key === "" ? -1 : a.label.localeCompare(b.label)));
}

/** Tasks of one column, in the order the column shows them. */
export function columnTasks(tasks: KanbanTask[], column: ColumnId): KanbanTask[] {
    const list = tasks.filter((t) => columnOf(t) === column);
    if (column === "backlog" || column === "specd") return list.sort(backlogOrder);
    // Newest change first everywhere else: what just moved is on top.
    return list.sort((a, b) => b.updated_at - a.updated_at);
}

/** Agents at work with no task on the board: shown as untracked cards (§6.6). */
export function untrackedAgents(agents: KanbanAgent[], tasks: KanbanTask[]): KanbanAgent[] {
    const holding = new Set(
        tasks
            .filter((t) => t.state === "claimed" || t.state === "blocked")
            .flatMap((t) => [t.claimed_by_uid, t.claimed_by].filter((x) => x))
    );
    return agents.filter(
        (a) => (a.state === "working" || a.state === "waiting") && !holding.has(a.uid ?? "") && !holding.has(a.name)
    );
}

/** What the Review column header says: how many, how many of them are specs, and how long the oldest
 *  has waited. */
export function reviewLoad(tasks: KanbanTask[], now: number): { count: number; specs: number; oldestMs: number } {
    const inReview = tasks.filter((t) => columnOf(t) === "review");
    const oldest = inReview.reduce((min, t) => Math.min(min, t.updated_at), now);
    return {
        count: inReview.length,
        specs: inReview.filter((t) => isResearchKind(t.kind)).length,
        oldestMs: inReview.length ? now - oldest : 0,
    };
}

/** Who asked for a task, in words. */
export function requestedBy(t: KanbanTask): string {
    const c = t.card;
    if (c.requested_by_kind === "operator") return c.filed_by ? `you, via ${c.filed_by}` : "you";
    if (c.requested_by_kind === "agent" && c.requested_by) return c.requested_by;
    return c.filed_by || t.created_by || "unknown";
}

/** The checklist's progress, "3/7", or null without one. */
export function checklistProgress(t: KanbanTask): string | null {
    const list = t.card.checklist ?? [];
    if (!list.length) return null;
    return `${list.filter((i) => i.status === "completed").length}/${list.length}`;
}

export interface RepoLane {
    /** The repository, `""` for tasks with none. */
    repo: string;
    label: string;
    tasks: KanbanTask[];
}

/** A repository's name as a lane or card shows it: `owner/repo`, or a local folder's last part. */
export function repoLabel(repo: string): string {
    if (!repo) return "No repository";
    return repo.startsWith("local:") ? repo.replace(/^local:.*[\\/]/, "") : repo;
}

/** The board split into one lane per repository (§6.11), those with unfinished work first, each by name;
 *  tasks with no repository last. */
export function repoLanes(tasks: KanbanTask[]): RepoLane[] {
    const byRepo = new Map<string, KanbanTask[]>();
    for (const t of tasks) {
        const list = byRepo.get(t.card.repo) ?? [];
        list.push(t);
        byRepo.set(t.card.repo, list);
    }
    const active = (l: KanbanTask[]) => l.some((t) => !isFinishedColumn(columnOf(t)));
    return [...byRepo.entries()]
        .map(([repo, list]) => ({ repo, label: repoLabel(repo), tasks: list }))
        .sort((a, b) => {
            if (!a.repo !== !b.repo) return a.repo ? -1 : 1;
            if (active(a.tasks) !== active(b.tasks)) return active(a.tasks) ? -1 : 1;
            return a.label.localeCompare(b.label);
        });
}

/** Review's soft limit from the settings (`kanban:reviewlimit`, D6): a positive whole number, else none (0). */
export function reviewLimitOf(settings: Record<string, unknown> | undefined): number {
    const v = settings?.["kanban:reviewlimit"];
    return typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.floor(v) : 0;
}

/** The tasks this one starts after that aren't finished yet: while any is, nobody takes it. */
export function waitingOn(t: KanbanTask): KanbanAfter[] {
    return (t.card.after ?? []).filter((a) => a.state !== "done" && a.state !== "cancelled");
}

/** "#12, 15" → ["12", "15"]: the numbers typed for "starts after". */
export function parseAfter(text: string): string[] {
    return text
        .split(/[\s,;]+/)
        .map((s) => s.replace(/^#/, ""))
        .filter(Boolean);
}

/** A PR link's label: "#12" for a GitHub PR address, else "PR". */
export function prLabel(url: string): string {
    const m = /\/pull\/(\d+)/.exec(url);
    return m ? `#${m[1]}` : "PR";
}

/** Whether a task matches the search box: title, number, agent, repository. */
export function matchesQuery(t: KanbanTask, q: string): boolean {
    const s = q.trim().toLowerCase();
    if (!s) return true;
    if (s.replace(/^#/, "") === String(t.card.num)) return true;
    return [t.title, t.target_agent, t.claimed_by, t.card.repo, t.card.branch, t.kind]
        .filter(Boolean)
        .some((f) => f.toLowerCase().includes(s));
}
