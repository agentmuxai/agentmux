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
function useStagedBoard(requested: () => string | null, asked: () => number) {
    const [chosen, setChosen] = createSignal<string | null>(null);
    // A new ask (the cog on a Kanban pane, each click with its own `asked`) wins over a board picked here before it. The
    // key is a memo, so a change to the pane's meta that isn't an ask leaves the pick alone.
    const ask = createMemo(() => `${requested() ?? ""}|${asked()}`);
    createEffect(on(ask, () => requested() && setChosen(requested()), { defer: true }));
    const model = new KanbanModel(() => chosen() ?? requested());
    model.setVisible(true);
    onCleanup(() => model.dispose());
    const boards = () => model.everything()?.boards ?? [];
    // A memo: the board's slug is the same on every poll, so nothing that reads it (the draft's reset) re-runs.
    const selected = createMemo(() => resolveBoard(boards(), chosen() ?? requested(), defaultSlugOf(model.everything())));
    const info = (): KanbanBoardInfo | undefined => infoOf(boards(), selected());
    // Likewise the stored stages: a poll that brings the same stages leaves the same array.
    const stored = createMemo(() => stagesOfBoard(info()), undefined, { equals: sameStages });
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
    // A board whose stored stages can't be used shows the default: applying the default is what repairs it.
    const dirty = () => draft() !== null && (!sameStages(draft() as KanbanStage[], b.stored()) || !!b.info()?.stages_error);
    const problems = createMemo(() => validateStages(stages()));
    const changedMeanwhile = () => dirty() && (b.info()?.stages_rev ?? 0) !== baseRev();
    const start = () => {
        if (draft() === null) setBaseRev(b.info()?.stages_rev ?? 0);
    };
    return { draft, setDraft, baseRev, setBaseRev, stages, dirty, problems, changedMeanwhile, start };
}

/** Send `set` to one board; true when srv took it. A set that is the default is a reset. */
async function sendStages(model: KanbanModel, slug: string, set: readonly KanbanStage[], rev: number | undefined): Promise<boolean> {
    const res = await model.act({
        action: "board_edit",
        board: slug,
        ...(sameStages(set, defaultStages()) ? { stages_reset: true } : { stages: toStageSet(set) }),
        ...(rev === undefined ? {} : { stages_rev: rev }),
    });
    return res !== null;
}

/** The operations the rows and buttons call: each one edits the draft. */
function stageEdits(edit: (f: (s: readonly KanbanStage[]) => KanbanStage[]) => void) {
    return {
        rename: (id: string, name: string) => edit((s) => renameStage(s, id, name)),
        recolour: (id: string, colour: StageColour) => edit((s) => recolourStage(s, id, colour)),
        move: (id: string, delta: number) => edit((s) => moveStage(s, id, delta)),
        toggle: (id: string, source: ColumnId, on: boolean) => edit((s) => toggleSource(s, id, source, on)),
        add: (name: string) => edit((s) => addStage(s, name)),
        remove: (id: string, into: string | null) => edit((s) => removeStage(s, id, into)),
        reset: () => edit(() => defaultStages()),
    };
}

/** `requested`: the board the pane was asked to show (the cog on a Kanban pane names it), by slug. */
export function createKanbanStagesState(requested: () => string | null, asked: () => number = () => 0) {
    const b = useStagedBoard(requested, asked);
    const { model } = b;
    const [busy, setBusy] = createSignal(false);
    const [copy, setCopy] = createSignal<CopyResult | null>(null);
    // Why the last Apply was refused: kept here, since the re-read after it clears the model's own error.
    const [refused, setRefused] = createSignal<string | null>(null);
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

    const send = (slug: string, set: readonly KanbanStage[], rev: number | undefined) => sendStages(model, slug, set, rev);

    const apply = async () => {
        if (!editable() || !dirty() || problems().length > 0) return;
        setBusy(true);
        setRefused(null);
        const ok = await send(b.selected(), stages(), d.baseRev());
        const why = ok ? null : model.error();
        setBusy(false);
        if (ok) return d.setDraft(null);
        // A refusal (a stale revision, a rule srv checks, a network failure) is shown until dismissed; read the board
        // again now, so a change made meanwhile shows as "Load theirs" or "Apply mine" at once.
        setRefused(why);
        await model.refresh();
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
        await model.refresh();
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
        ...stageEdits(edit),
        removeInto: (id: string) => removalTarget(stages(), id),
        refused,
        dismissRefused: () => {
            setRefused(null);
            model.dismissError();
        },
        discard: () => {
            d.setDraft(null);
            setRefused(null);
        },
        apply,
        applyMine,
        copyToAll,
    };
}

export type KanbanStagesState = ReturnType<typeof createKanbanStagesState>;
