// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The palette's zoom commands for a pane with parts: "Zoom in file tree", "Reset editor zoom", "Reset zoom
 * in file tree and editor". The generic "Zoom In" / "Zoom Out" / "Actual Size" act on the part that has the
 * keyboard (`zoomSubjectForKeys`); these name a part, and are listed only while the selected pane is that
 * view and the part is on screen. A pane that declares parts (`paneZoom.parts`) adds a row here.
 */

import type { CommandEntry } from "./command-registry";
import { getBlockComponentModel, getFocusedBlockId } from "./block-component-registry";
import { MAIN_PART, zoomPartElement } from "./zoom-parts";
import { zoomIn, zoomOut, zoomPaneReset, zoomReset } from "./zoom";

interface PartRow {
    view: string;
    part: string;
    /** How the part reads in a command: "Zoom in <noun>". */
    noun: string;
}

const ROWS: readonly PartRow[] = [
    { view: "editor", part: "tree", noun: "file tree" },
    { view: "editor", part: MAIN_PART, noun: "editor" },
];

/** The words after "Reset zoom in" for a view's parts, in order: "file tree and editor". */
const bothNouns = (view: string): string => ROWS.filter((r) => r.view === view).map((r) => r.noun).join(" and ");

function selectedPaneShows(view: string, part: string): boolean {
    const blockId = getFocusedBlockId();
    if (!blockId || getBlockComponentModel(blockId)?.viewModel?.viewType !== view) return false;
    return part === MAIN_PART || zoomPartElement(blockId, part) != null;
}

export function zoomPartCommands(): CommandEntry[] {
    const entries: CommandEntry[] = ROWS.flatMap(({ view, part, noun }) => {
        const when = () => selectedPaneShows(view, part);
        const base = { category: "View", when, keywords: "zoom size text" };
        return [
            { ...base, id: `view:zoom:${view}:${part}:in`, label: `Zoom in ${noun}`, icon: "magnifying-glass-plus", execute: () => zoomIn(undefined, part) },
            { ...base, id: `view:zoom:${view}:${part}:out`, label: `Zoom out ${noun}`, icon: "magnifying-glass-minus", execute: () => zoomOut(undefined, part) },
            { ...base, id: `view:zoom:${view}:${part}:reset`, label: `Reset ${noun} zoom`, icon: "magnifying-glass", execute: () => zoomReset(part) },
        ];
    });
    for (const view of new Set(ROWS.map((r) => r.view))) {
        entries.push({
            id: `view:zoom:${view}:resetBoth`,
            label: `Reset zoom in ${bothNouns(view)}`,
            category: "View",
            icon: "magnifying-glass",
            keywords: "zoom size actual",
            when: () => selectedPaneShows(view, MAIN_PART),
            execute: () => zoomPaneReset(),
        });
    }
    return entries;
}
