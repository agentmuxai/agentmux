// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Needs-you count on a widget-bar button that opens the Swarm's Kanban section
// (SPEC_WARDEN_KANBAN_2026_10_20.md §6.13: "Needs you is the only interruption"). The same number as
// the Swarm's title on its Kanban section; hidden at zero.

import { Show, type JSX } from "solid-js";

import { movedMeta } from "@/app/block/pane-sections";

import { useKanbanNeeds } from "./kanban-held";

import "./kanban-needs-badge.scss";

/** Whether a widget-bar entry opens the Swarm on its Kanban section (the built-in Kanban button, or a
 *  section entry the user pinned). An entry that still names Warden's Kanban opens there too
 *  (MOVED_SECTIONS), so it counts. */
export function opensKanban(widget: WidgetConfigType | undefined): boolean {
    const meta = movedMeta(widget?.blockdef?.meta as Record<string, unknown> | undefined);
    return meta?.view === "swarm" && meta?.["swarm:section"] === "kanban";
}

export function KanbanNeedsBadge(): JSX.Element {
    const needs = useKanbanNeeds();
    return (
        <Show when={needs() > 0}>
            <span class="kanban-needs-badge" role="status" aria-label={`${needs()} need${needs() === 1 ? "s" : ""} you`}>
                {needs() > 99 ? "99+" : needs()}
            </span>
        </Show>
    );
}
