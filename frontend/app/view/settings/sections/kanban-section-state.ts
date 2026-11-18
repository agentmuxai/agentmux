// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The state of Settings → Kanban (kanban-section.tsx): the board whose stages are edited, a draft of them, and
// what Apply, Discard, Reset and "Use for all my boards" do (docs/specs/SPEC_KANBAN_CUSTOM_STAGES_2026_11_16.md
// §8.2). The editor works on a copy: the rules span rows, so nothing is sent until the person applies it.

import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";

import { defaultSlugOf, infoOf, resolveBoard } from "@/app/view/kanban/kanban-boards";
import { KanbanModel } from "@/app/view/kanban/kanban-model";
import {
    addStage,
    canAdd,
    moveStage,
    recolourStage,
    removalTarget,
    removeStage,
    renameStage,
    sameStages,
    toggleSource,
} from "@/app/view/kanban/kanban-stages-draft";
import {
    defaultStages,
    stageCounts,
    stagesOfBoard,
    toStageSet,
    validateStages,
    type KanbanStage,
    type StageColour,
} from "@/app/view/kanban/kanban-stages";
import type { ColumnId } from "@/app/view/kanban/kanban-columns";
import type { KanbanBoardInfo } from "@/app/view/kanban/kanban-types";

/** What "Use for all my boards" did: the boards it copied to, and those it could not. */
export interface CopyResult {
    copied: number;
    skipped: string[];
}

/** The board whose stages are edited, and what srv says its stages are. `requested`: the board the pane was asked to
 *  show (the cog on a Kanban pane names it), by slug. */
function useStagedBoard(requested: () => string | null) {
    const [chosen, setChosen] = createSignal<string | null>(null);
    // A new ask (the cog on a Kanban pane) wins over a board picked here before it.
    createEffect(on(requested, (r) => r && setChosen(r), { defer: true }));
    const model = new KanbanModel(() => chosen() ?? requested());
    model.setVisible(true);
    onCleanup(() => model.dispose());
    const boards = () => model.everything()?.boards ?? [];
    const selected = () => resolveBoard(boards(), chosen() ?? requested(), defaultSlugOf(model.everything()));
    const info = (): KanbanBoardInfo | undefined => infoOf(boards(), selected());
    const stored = createMemo(() => stagesOfBoard(info()));
    // An older srv sends no stages: the editor says so and keeps Apply off.
    const supported = () => !model.everything() || !!info()?.stages;
    return { model, boards, selected, select: setChosen, info, stored, supported };
}

/** The draft the person edits, kept apart from what is stored until it is applied. */
function useDraft(b: ReturnType<typeof useStagedBoard>, onSwitch: () => void) {
    const [draft, setDraft] = createSignal<KanbanStage[] | null>(null);
    // The revision the draft was started from: sent with Apply, so a change made meanwhile is told, not overwritten.
    const [baseRev, setBaseRev] = createSignal(0);
    createEffect(
        on(b.selected, () => {
            setDraft(null);
            onSwitch();
        }, { defer: true })
    );
    const stages = () => draft() ?? b.stored();
    const dirty = () => draft() !== null && !sameStages(draft() as KanbanStage[], b.stored());
    const problems = createMemo(() => validateStages(stages()));
    const changedMeanwhile = () => dirty() && (b.info()?.stages_rev ?? 0) !== baseRev();
    const start = () => {
        if (draft() === null) setBaseRev(b.info()?.stages_rev ?? 0);
    };
    return { draft, setDraft, baseRev, setBaseRev, stages, dirty, problems, changedMeanwhile, start };
}

/** `requested`: the board the pane was asked to show (the cog on a Kanban pane names it), by slug. */
export function createKanbanStagesState(requested: () => string | null) {
    const b = useStagedBoard(requested);
    const { model } = b;
    const [busy, setBusy] = createSignal(false);
    const [copy, setCopy] = createSignal<CopyResult | null>(null);
    const d = useDraft(b, () => setCopy(null));
    const { stages, dirty, problems } = d;
    const archived = () => b.info()?.archived === true;
    const editable = () => b.supported() && !archived() && !busy();
    const counts = createMemo(() => stageCounts(model.board()?.tasks ?? [], stages()));

    const edit = (f: (s: readonly KanbanStage[]) => KanbanStage[]) => {
        if (!editable()) return;
        d.start();
        d.setDraft(f(stages()));
        setCopy(null);
    };

    /** Send `set` to one board; true when srv took it. A set that is the default is a reset. */
    const send = async (slug: string, set: readonly KanbanStage[], rev: number | undefined): Promise<boolean> => {
        const res = await model.act({
            action: "board_edit",
            board: slug,
            ...(sameStages(set, defaultStages()) ? { stages_reset: true } : { stages: toStageSet(set) }),
            ...(rev === undefined ? {} : { stages_rev: rev }),
        });
        return res !== null;
    };

    const apply = async () => {
        if (!editable() || !dirty() || problems().length > 0) return;
        setBusy(true);
        const ok = await send(b.selected(), stages(), d.baseRev());
        setBusy(false);
        if (ok) d.setDraft(null);
    };

    /** Apply after a change made meanwhile, on top of it. */
    const applyMine = async () => {
        d.setBaseRev(b.info()?.stages_rev ?? 0);
        await apply();
    };

    const copyToAll = async () => {
        if (!editable() || problems().length > 0) return;
        setBusy(true);
        const result: CopyResult = { copied: 0, skipped: [] };
        for (const other of b.boards()) {
            if (other.slug === b.selected()) continue;
            if (other.archived || !other.stages) result.skipped.push(other.name);
            else if (await send(other.slug, stages(), undefined)) result.copied += 1;
            else result.skipped.push(other.name);
        }
        // The board being edited too: what is on screen is what every board now has.
        if (dirty() && (await send(b.selected(), stages(), d.baseRev()))) d.setDraft(null);
        setBusy(false);
        setCopy(result);
    };

    return {
        model,
        boards: b.boards,
        selected: b.selected,
        select: b.select,
        info: b.info,
        supported: b.supported,
        stages,
        stored: b.stored,
        dirty,
        problems,
        changedMeanwhile: d.changedMeanwhile,
        archived,
        editable,
        busy,
        counts,
        copy,
        otherLiveBoards: () => b.boards().filter((o) => o.slug !== b.selected() && !o.archived).length,
        canAdd: () => canAdd(stages()),
        rename: (id: string, name: string) => edit((s) => renameStage(s, id, name)),
        recolour: (id: string, colour: StageColour) => edit((s) => recolourStage(s, id, colour)),
        move: (id: string, delta: number) => edit((s) => moveStage(s, id, delta)),
        toggle: (id: string, source: ColumnId, on: boolean) => edit((s) => toggleSource(s, id, source, on)),
        add: (name: string) => edit((s) => addStage(s, name)),
        removeInto: (id: string) => removalTarget(stages(), id),
        remove: (id: string, into: string | null) => edit((s) => removeStage(s, id, into)),
        reset: () => edit(() => defaultStages()),
        discard: () => d.setDraft(null),
        apply,
        applyMine,
        copyToAll,
    };
}

export type KanbanStagesState = ReturnType<typeof createKanbanStagesState>;
