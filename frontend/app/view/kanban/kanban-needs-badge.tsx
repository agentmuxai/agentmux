// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Needs-you count on a widget-bar button that opens the Kanban pane
// (SPEC_WARDEN_KANBAN_2026_10_20.md §6.13: "Needs you is the only interruption"). The same number as
// the Kanban pane's title; hidden at zero.

import { Show, type JSX } from "solid-js";

import { movedMeta } from "@/app/block/pane-sections";

import { useKanbanNeeds } from "./kanban-held";

import "./kanban-needs-badge.scss";

/** Whether a widget-bar entry opens the Kanban pane (the built-in Kanban button, or an entry the user
 *  wrote). An entry that still names the Kanban as a section of Warden or the Swarm opens it too
 *  (MOVED_SECTIONS), so it counts. */
export function opensKanban(widget: WidgetConfigType | undefined): boolean {
    const meta = movedMeta(widget?.blockdef?.meta as Record<string, unknown> | undefined);
    return meta?.view === "kanban";
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
