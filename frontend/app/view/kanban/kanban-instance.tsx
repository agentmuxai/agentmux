// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Kanban pane's instance, loaded with the first one (kanban.tsx). The board is the same one the Swarm
// showed in its Kanban section, with the same pane meta: `kanban:open` (a task to open), `kanban:lanes`
// and `kanban:collapsed`.

import { createMemo, createSignal } from "solid-js";

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { readZoom } from "@/app/store/zoom-factor";

import { parseCollapsed } from "./kanban-columns";
import { KanbanSection } from "./kanban-section";

/** The pane's title: the Kanban, and how many tasks need the operator (SPEC_WARDEN_KANBAN_2026_10_20.md §6.1). */
export function kanbanTitleOf(needs: number): string {
    return needs > 0 ? `Kanban · ${needs} need${needs === 1 ? "s" : ""} you` : "Kanban";
}

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    // Set by the board once it is mounted; it names the pane.
    const [needs, setNeeds] = createSignal(0);
    const open = createMemo<string | null>(() => {
        const id = ctx.meta()?.["kanban:open"];
        return typeof id === "string" && id ? id : null;
    });
    return {
        component: () => (
            <div class="kanban-pane" style={{ zoom: readZoom(ctx.meta()) }}>
                <KanbanSection
                    visible={ctx.visibility() === "active"}
                    openRequest={open()}
                    onOpenHandled={() => void ctx.setMeta({ "kanban:open": null })}
                    onNeedsCount={setNeeds}
                    lanes={ctx.meta()?.["kanban:lanes"] === true}
                    onLanes={(on) => void ctx.setMeta({ "kanban:lanes": on })}
                    collapsed={parseCollapsed(ctx.meta()?.["kanban:collapsed"])}
                    onCollapsed={(ids) => void ctx.setMeta({ "kanban:collapsed": ids })}
                />
            </div>
        ),
        liveTitle: () => ({ text: kanbanTitleOf(needs()) }),
    };
}
