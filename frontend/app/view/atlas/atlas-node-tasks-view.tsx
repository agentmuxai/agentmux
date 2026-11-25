// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a node says of its agent's tasks (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.3): a line each under the name and
 * state, the task as a link to its card on the Kanban and its pull request as a second one; or, where the map is too
 * small to read them, how many there are. A click on a link is the link's: it neither opens the node's card nor
 * starts a drag (a press on a node is not on the map's background, atlas-pan-zoom.ts `NOT_BACKGROUND`).
 */

import { openTaskLink } from "@/app/element/link-destinations";
import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { openLink } from "@/app/store/misc-utils";
import { For, Show, type JSX } from "solid-js";
import type { AtlasTask, NodeTasks } from "./atlas-work";

/** How many tasks the agent has, in the node's name line. */
export function TaskCount(props: { total: number }): JSX.Element {
    return (
        <span
            class="atlas-node__count"
            data-testid="atlas-node-count"
            aria-label={t("atlas.node.tasks", { n: props.total })}
        >
            <i class="fa-solid fa-list-check" aria-hidden="true" />
            {props.total}
        </span>
    );
}

/** The task's own colour on the board: the mark before its line. */
const stageVar = (task: AtlasTask) => `var(--kanban-stage-${task.column})`;

function TaskLine(props: { task: AtlasTask }): JSX.Element {
    const task = () => props.task;
    return (
        <li
            class="atlas-node__task-line"
            classList={{ "atlas-node__task-line--waiting": !task().held }}
            data-testid="atlas-node-task"
            data-held={task().held}
            style={{ "--atlas-task-stage": stageVar(task()) }}
        >
            <span class="atlas-node__task-mark" aria-hidden="true" />
            <Button
                variant="link"
                opens
                class="atlas-node__task"
                {...tip(`#${task().num} ${task().title}`)}
                onClick={(e) => {
                    e.stopPropagation();
                    void openTaskLink(task().num);
                }}
            >
                #{task().num} {task().title}
            </Button>
            <Show when={task().pr}>
                {(pr) => (
                    <Button
                        variant="link"
                        opens
                        class="atlas-node__task-pr"
                        onClick={(e) => {
                            e.stopPropagation();
                            void openLink(pr().url);
                        }}
                    >
                        {pr().label}
                    </Button>
                )}
            </Show>
        </li>
    );
}

/** The lines of a node's tasks, and one that says how many more there are (a click on it opens the card, which lists them all). */
export function NodeTasksList(props: { tasks: NodeTasks }): JSX.Element {
    return (
        <ul class="atlas-node__tasks" data-testid="atlas-node-tasks">
            <For each={props.tasks.shown}>{(task) => <TaskLine task={task} />}</For>
            <Show when={props.tasks.more > 0}>
                <li class="atlas-node__task-line atlas-node__more" data-testid="atlas-node-more">
                    {t("atlas.node.moreTasks", { n: props.tasks.more })}
                </li>
            </Show>
        </ul>
    );
}
