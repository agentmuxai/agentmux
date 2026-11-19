// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's stages (docs/specs/SPEC_KANBAN_CUSTOM_STAGES_2026_11_16.md §4, §5): the named, coloured
// columns the person sets. A stage shows some of the board's eight sources (the built-in columns, which
// `columnOf` still computes from a task's state, kind, PR and holder); a task lands in the stage that shows
// its source. Pure: the board, the Settings editor and their tests share it. The rules mirror srv's
// (`crates/srv-core/src/backend/storage/work_board_stages.rs`), which refuses a set that breaks one.

import {
    backlogOrder,
    columnOf,
    COLUMNS,
    isFinishedColumn,
    type ColumnId,
} from "./kanban-columns";
import type { KanbanBoardInfo, KanbanTask } from "./kanban-types";

/** The colours a stage may use: names of theme colours, never a hex (spec §9). */
export type StageColour = "grey" | "cyan" | "violet" | "magenta" | "amber" | "yellow" | "green" | "blue";

/** The palette, with the source whose `--kanban-stage-<source>` theme token each name is an alias of. */
export const STAGE_PALETTE: readonly { id: StageColour; token: ColumnId }[] = [
    { id: "grey", token: "backlog" },
    { id: "cyan", token: "research" },
    { id: "violet", token: "specd" },
    { id: "magenta", token: "progress" },
    { id: "amber", token: "needs" },
    { id: "yellow", token: "review" },
    { id: "green", token: "merged" },
    { id: "blue", token: "released" },
];

export interface KanbanStage {
    /** The default set's ids are the source ids; a new stage's is `st-` and six base-36 characters. */
    id: string;
    name: string;
    colour: StageColour;
    /** The sources it shows, at least one. */
    takes: ColumnId[];
}

export interface KanbanStageSet {
    version: 1;
    stages: KanbanStage[];
}

export const MIN_STAGES = 2;
export const MAX_STAGES = 8;
export const MAX_STAGE_NAME = 24;
/** The source that asks the operator for something: its stage stays whole. */
export const NEEDS_SOURCE: ColumnId = "needs";

/** Today's board: one stage per source, with today's names and colours. */
export function defaultStages(): KanbanStage[] {
    return COLUMNS.map((c) => ({
        id: c.id,
        name: c.label,
        colour: STAGE_PALETTE.find((p) => p.token === c.id)?.id ?? "grey",
        takes: [c.id],
    }));
}

/** A problem with one stage (`stage` is its id) or with the set (`stage` absent), in words for the person. */
export interface StageProblem {
    stage?: string;
    message: string;
}

const SOURCE_IDS = COLUMNS.map((c) => c.id);
const nameKey = (name: string) => name.trim().toLowerCase();

function nameProblem(s: KanbanStage, others: readonly KanbanStage[]): string | null {
    const name = s.name.trim();
    if (!name) return "Give it a name";
    if ([...name].length > MAX_STAGE_NAME) return `A name is at most ${MAX_STAGE_NAME} characters`;
    // eslint-disable-next-line no-control-regex
    if (/[\u0000-\u001f\u007f]/.test(name)) return "A name can't hold a control character";
    if (others.some((o) => o !== s && nameKey(o.name) === nameKey(name))) return "Another stage has this name";
    if (nameKey(name) === "needs you" && !s.takes.includes(NEEDS_SOURCE)) {
        return "“Needs you” is the name of the stage that shows what needs you";
    }
    return null;
}

function sourceProblems(stages: readonly KanbanStage[]): StageProblem[] {
    const out: StageProblem[] = [];
    for (const src of SOURCE_IDS) {
        const shown = stages.filter((s) => s.takes.includes(src));
        const label = COLUMNS.find((c) => c.id === src)?.label ?? src;
        if (shown.length === 0) out.push({ message: `No stage shows ${label}` });
        if (shown.length > 1) shown.forEach((s) => out.push({ stage: s.id, message: `${label} is shown by another stage too` }));
    }
    for (const s of stages) {
        if (s.takes.includes(NEEDS_SOURCE) && s.takes.length > 1) {
            out.push({ stage: s.id, message: "This stage shows only what needs you" });
        }
    }
    return out;
}

/** Every rule of the spec (§5) a set breaks, each beside the stage it concerns when it concerns one. Empty
 *  means the set can be applied. */
export function validateStages(stages: readonly KanbanStage[]): StageProblem[] {
    const out: StageProblem[] = [];
    if (stages.length < MIN_STAGES) out.push({ message: `A board needs at least ${MIN_STAGES} stages` });
    if (stages.length > MAX_STAGES) out.push({ message: `A board has at most ${MAX_STAGES} stages` });
    for (const s of stages) {
        const why = nameProblem(s, stages);
        if (why) out.push({ stage: s.id, message: why });
        if (s.takes.length === 0) out.push({ stage: s.id, message: "Pick at least one thing for it to show" });
    }
    return [...out, ...sourceProblems(stages)];
}

/** A stored colour this build doesn't know draws as grey (a future palette), never as a missing colour. */
function knownColour(value: unknown): StageColour {
    return STAGE_PALETTE.some((p) => p.id === value) ? (value as StageColour) : "grey";
}

/** The stages srv sent for a board, ready to draw: its set when it is whole, else the default eight. An
 *  unknown colour is grey and an unknown source is ignored; a set that then breaks a rule is not used (the
 *  editor says why, and nothing is overwritten until the person acts). */
export function stagesOfBoard(info: Pick<KanbanBoardInfo, "stages" | "stages_error"> | undefined): KanbanStage[] {
    const raw = info?.stages;
    if (!raw || info?.stages_error || raw.version !== 1 || !Array.isArray(raw.stages)) return defaultStages();
    const stages: KanbanStage[] = raw.stages.map((s) => ({
        id: String(s.id),
        name: String(s.name),
        colour: knownColour(s.colour),
        takes: (Array.isArray(s.takes) ? s.takes : []).filter((t): t is ColumnId => SOURCE_IDS.includes(t as ColumnId)),
    }));
    return validateStages(stages).length === 0 ? stages : defaultStages();
}

/** The stage that shows a source. A set that passed `validateStages` has exactly one. */
export function stageOfSource(stages: readonly KanbanStage[], source: ColumnId): KanbanStage {
    return stages.find((s) => s.takes.includes(source)) ?? stages[0];
}

/** The stage a task is in: the one that shows its source. */
export function stageOfTask(t: KanbanTask, stages: readonly KanbanStage[]): KanbanStage {
    return stageOfSource(stages, columnOf(t));
}

/** A stage of waiting sources only (Backlog, Spec'd): its cards group by the agent they wait for, and it takes drops. */
export function isWaitingStage(stage: KanbanStage): boolean {
    return stage.takes.length > 0 && stage.takes.every((s) => s === "backlog" || s === "specd");
}

/** A stage of finished sources only (Merged, Released): the narrow list folds it. */
export function isFinishedStage(stage: KanbanStage): boolean {
    return stage.takes.length > 0 && stage.takes.every(isFinishedColumn);
}

/** The tasks a stage shows, in the order it shows them. */
export function stageTasks(tasks: readonly KanbanTask[], stage: KanbanStage): KanbanTask[] {
    const list = tasks.filter((t) => stage.takes.includes(columnOf(t)));
    return isWaitingStage(stage) ? list.sort(backlogOrder) : list.sort((a, b) => b.updated_at - a.updated_at);
}

/** The ids in `value` that are stages, once each, in stage order: what a pane's `kanban:collapsed` meta holds.
 *  A stage a board doesn't have is ignored here and never rewritten, so switching boards and back finds it. */
export function parseCollapsedStages(value: unknown, stages: readonly KanbanStage[]): string[] {
    if (!Array.isArray(value)) return [];
    return stages.map((s) => s.id).filter((id) => value.includes(id));
}

/** The pane's collapsed set with `id` toggled. Ids of stages another board has are kept as they were. */
export function toggleCollapsedStage(stored: unknown, stages: readonly KanbanStage[], id: string): string[] {
    const all = Array.isArray(stored) ? stored.filter((x): x is string => typeof x === "string") : [];
    const mine = new Set(stages.map((s) => s.id));
    const elsewhere = all.filter((x) => !mine.has(x));
    const open = parseCollapsedStages(all, stages);
    const next = open.includes(id) ? open.filter((x) => x !== id) : [...open, id];
    return [...elsewhere, ...parseCollapsedStages(next, stages)];
}

/** How many tasks each stage holds, by stage id (the editor's preview). */
export function stageCounts(tasks: readonly KanbanTask[], stages: readonly KanbanStage[]): Record<string, number> {
    const out: Record<string, number> = Object.fromEntries(stages.map((s) => [s.id, 0]));
    for (const t of tasks) out[stageOfTask(t, stages).id] += 1;
    return out;
}

/** The stage set as it is sent to srv. */
export function toStageSet(stages: readonly KanbanStage[]): KanbanStageSet {
    return { version: 1, stages: stages.map((s) => ({ ...s, name: s.name.trim(), takes: [...s.takes] })) };
}
