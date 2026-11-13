// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban pane's board (it was a Warden section, then a Swarm section, SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §4):
// what work there is and where it stands, kept current by the agents doing it
// (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md). Eight columns
// (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md): Backlog and Spec'd (grouped by the agent each
// task waits for), In Research, In progress, Needs you, Review (its PR in review), Merged, Released.
// Agents file and move tasks with the `Tasks` tool; srv moves them on what it knows (an agent asks,
// errors, closes; a PR merges; a release ships it); the operator starts, answers, accepts and sends back
// here.

import { Show, type JSX } from "solid-js";

import { IconButton } from "@/app/element/ui";

import { AgentStrip, BoardArea, KanbanToolbar, OpenDrawer } from "./kanban-section-parts";
import { createKanbanSection, type SectionProps } from "./kanban-section-state";
import { KanbanBoardsPanel } from "./kanban-boards-panel";
import { KanbanNewTask } from "./kanban-new-task";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./kanban.scss";

/** What the Kanban pane passes (kanban-props.ts); all but `visible` are optional, so the board also mounts alone. */
export const KanbanSection = (props: SectionProps): JSX.Element => {
    const s = createKanbanSection(props);
    return (
        <div ref={s.bindContainer} class="kanban-container">
            <div class="warden-manager-body kanban">
                <KanbanToolbar s={s} />

                <Show when={s.model.error()}>
                    {(e) => (
                        <div class="warden-manager-error kanban-error" role="alert">
                            {e()}
                            <IconButton icon="xmark" label="Dismiss" onClick={() => s.model.dismissError()} />
                        </div>
                    )}
                </Show>

                <Show when={s.managing()}>
                    <KanbanBoardsPanel s={s} />
                </Show>

                <Show when={s.adding()}>
                    <KanbanNewTask model={s.model} agents={s.agents()} onDone={() => s.setAdding(false)} />
                </Show>

                <AgentStrip s={s} />

                <BoardArea s={s} />

                <OpenDrawer s={s} />
            </div>
        </div>
    );
};
