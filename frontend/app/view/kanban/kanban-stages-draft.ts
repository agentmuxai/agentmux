// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The edits the Kanban stages editor makes to its draft (SPEC_KANBAN_CUSTOM_STAGES_2026_11_16.md §8.2): each
// returns a new list and leaves the rules to `validateStages`, which says what is still wrong. Pure.

import type { ColumnId } from "./kanban-columns";
import { MAX_STAGES, NEEDS_SOURCE, STAGE_PALETTE, type KanbanStage, type StageColour } from "./kanban-stages";

const BASE36 = "0123456789abcdefghijklmnopqrstuvwxyz";

/** An id for a new stage: `st-` and six base-36 characters. `rand` is for tests. */
export function newStageId(rand: () => number = Math.random): string {
    let out = "st-";
    for (let i = 0; i < 6; i++) out += BASE36[Math.floor(rand() * 36)];
    return out;
}

const replace = (stages: readonly KanbanStage[], id: string, f: (s: KanbanStage) => KanbanStage): KanbanStage[] =>
    stages.map((s) => (s.id === id ? f(s) : s));

export function renameStage(stages: readonly KanbanStage[], id: string, name: string): KanbanStage[] {
    return replace(stages, id, (s) => ({ ...s, name }));
}

export function recolourStage(stages: readonly KanbanStage[], id: string, colour: StageColour): KanbanStage[] {
    return replace(stages, id, (s) => ({ ...s, colour }));
}

/** Move a stage `delta` places (negative is earlier); it stops at the ends. */
export function moveStage(stages: readonly KanbanStage[], id: string, delta: number): KanbanStage[] {
    const from = stages.findIndex((s) => s.id === id);
    const to = Math.min(stages.length - 1, Math.max(0, from + delta));
    if (from < 0 || to === from) return [...stages];
    const next = [...stages];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    return next;
}

/** Whether a stage may show `source` in the editor: what needs the operator is a stage of its own, so no other
 *  stage offers it, and its stage offers nothing else. */
export function canShow(stage: KanbanStage, source: ColumnId): boolean {
    return stage.takes.includes(NEEDS_SOURCE) ? source === NEEDS_SOURCE : source !== NEEDS_SOURCE;
}

/** `stage` shows `source` (or stops showing it). A source is shown by one stage, so turning it on takes it from the
 *  stage that had it, which is left to say it shows nothing (the person gives it something or removes it). */
export function toggleSource(stages: readonly KanbanStage[], id: string, source: ColumnId, on: boolean): KanbanStage[] {
    const target = stages.find((s) => s.id === id);
    if (!target || !canShow(target, source)) return [...stages];
    return stages.map((s) => {
        if (s.id === id) return { ...s, takes: on ? [...s.takes.filter((t) => t !== source), source] : s.takes.filter((t) => t !== source) };
        return on ? { ...s, takes: s.takes.filter((t) => t !== source) } : s;
    });
}

/** The first palette colour no stage uses, else grey. */
function freeColour(stages: readonly KanbanStage[]): StageColour {
    return STAGE_PALETTE.find((p) => !stages.some((s) => s.colour === p.id))?.id ?? "grey";
}

/** Whether another stage can be added. */
export function canAdd(stages: readonly KanbanStage[]): boolean {
    return stages.length < MAX_STAGES;
}

/** A new stage at the end: it shows nothing yet, so Apply stays off until the person picks something for it. */
export function addStage(stages: readonly KanbanStage[], name: string, id: string = newStageId()): KanbanStage[] {
    if (!canAdd(stages)) return [...stages];
    return [...stages, { id, name, colour: freeColour(stages), takes: [] }];
}

/** Whether a stage can be removed: not the one that shows what needs the operator. */
export function canRemove(stages: readonly KanbanStage[], id: string): boolean {
    const s = stages.find((x) => x.id === id);
    return !!s && !s.takes.includes(NEEDS_SOURCE);
}

/** Where a removed stage's sources go by default: the stage before it, the next one for the first, never the one
 *  that shows what needs the operator. Null when it shows nothing, or no other stage can take them. */
export function removalTarget(stages: readonly KanbanStage[], id: string): string | null {
    const at = stages.findIndex((s) => s.id === id);
    if (at < 0 || stages[at].takes.length === 0) return null;
    const ok = (s: KanbanStage) => s.id !== id && !s.takes.includes(NEEDS_SOURCE);
    const before = stages.slice(0, at).reverse().find(ok);
    return (before ?? stages.slice(at + 1).find(ok))?.id ?? null;
}

/** Remove a stage; what it showed is shown by `into` instead (when it shows anything). */
export function removeStage(stages: readonly KanbanStage[], id: string, into: string | null): KanbanStage[] {
    const gone = stages.find((s) => s.id === id);
    if (!gone || !canRemove(stages, id)) return [...stages];
    return stages
        .filter((s) => s.id !== id)
        .map((s) => (s.id === into ? { ...s, takes: [...s.takes, ...gone.takes.filter((t) => !s.takes.includes(t))] } : s));
}

/** Whether two lists are the same stages in the same order: the draft against what is stored. */
export function sameStages(a: readonly KanbanStage[], b: readonly KanbanStage[]): boolean {
    return (
        a.length === b.length &&
        a.every((s, i) => s.id === b[i].id && s.name === b[i].name && s.colour === b[i].colour && s.takes.join() === b[i].takes.join())
    );
}
