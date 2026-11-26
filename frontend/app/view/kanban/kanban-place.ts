// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Kanban's place in the window's trail (store/pane-history.ts): its board and open card are what Back and
// Forward put back. What the person clicks records a step; a link's request, a restore and data arriving do not.
// SPEC_BACK_FORWARD_FOLLOW_THE_WHOLE_TRAIL_2026_11_26.md.

import { mergeProps, onCleanup } from "solid-js";

import { recordLinkedPlace, recordPlaceMove } from "@/app/store/pane-history";
import { registerPlaceSource } from "@/app/store/place-sources";

import { taskForOpenRequestAnywhere } from "./kanban-boards";
import type { KanbanSectionState, SectionProps } from "./kanban-section-state";
import type { KanbanTask } from "./kanban-types";

/**
 * `props` with the end of a link's request noted for the trail: the card a link opens is shown after the
 * link's gesture has recorded its place, and belongs to that step.
 */
export function placedProps(props: SectionProps): SectionProps {
    const blockId = props.placeBlockId;
    if (!blockId) return props;
    return mergeProps(props, {
        onOpenHandled: () => {
            props.onOpenHandled?.();
            // The card is shown once the effect that took the request is done.
            setTimeout(() => recordLinkedPlace(blockId), 0);
        },
    });
}

/**
 * The section's state with its place joined to the window's trail: the pane's place source is registered, and the
 * setters the person's clicks use (open or close a card, pick a board) move the view and then record the step.
 * The raw setters stay what a link's request and a restore use, so those record nothing.
 */
export function withKanbanPlace(base: KanbanSectionState, props: SectionProps): KanbanSectionState {
    const blockId = props.placeBlockId;
    if (!blockId) return base;
    const moved = () => recordPlaceMove(blockId);
    const openCard = (id: string | null) => {
        base.setOpenId(id);
        moved();
    };
    const pickBoard = (slug: string) => {
        base.selectBoard(slug);
        moved();
    };
    onCleanup(
        registerPlaceSource(blockId, {
            read: () => ({ board: base.selectedSlug() || undefined, task: base.openTask()?.id }),
            settled: () => !props.openRequest,
            restore: (place) => {
                const board = typeof place.board === "string" ? place.board : undefined;
                const task = typeof place.task === "string" ? place.task : undefined;
                if (board && board !== base.selectedSlug()) base.selectBoard(board);
                if (!task) {
                    base.setOpenId(null);
                    return true;
                }
                // Loaded and the task is on no board: it was deleted. Not loaded yet: show it when it arrives.
                const everything = base.model.everything();
                if (everything && !taskForOpenRequestAnywhere(everything, task)) return false;
                base.setOpenId(task);
                return true;
            },
        })
    );
    // What the views read of these three is that they open, pick and answer nothing the person's click needs.
    const clicks = { setOpenId: openCard, selectBoard: pickBoard, open: (t: KanbanTask) => openCard(t.id) };
    return { ...base, ...(clicks as unknown as Pick<KanbanSectionState, keyof typeof clicks>) };
}
