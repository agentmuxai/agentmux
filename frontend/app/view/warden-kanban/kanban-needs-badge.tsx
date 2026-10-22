// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Needs-you count on a widget-bar button that opens Warden's Kanban section
// (SPEC_WARDEN_KANBAN_2026_10_20.md §6.13: "Needs you is the only interruption"). The same number as
// the Warden tab title; hidden at zero.

import { Show, type JSX } from "solid-js";

import { useKanbanNeeds } from "./kanban-held";

import "./kanban-needs-badge.scss";

/** Whether a widget-bar entry opens Warden on its Kanban section (the built-in Kanban button, or a
 *  section entry the user pinned). */
export function opensKanban(widget: WidgetConfigType | undefined): boolean {
    const meta = widget?.blockdef?.meta as Record<string, unknown> | undefined;
    return meta?.view === "warden" && meta?.["warden:section"] === "kanban";
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
