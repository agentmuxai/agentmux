// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The card's rows for where an agent runs and what it is on (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.3):
 * host or container (and the container itself, §6.4), the account of an agent of another machine, the branch of
 * its clone, and the Kanban tasks it holds with their pull requests. Each row shows only when its fact is known.
 */

import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import { openLink } from "@/app/store/misc-utils";
import { clockTime } from "@/app/store/muxbus-delivery";
import { RuntimeBadge } from "@/app/view/agent/components/RuntimeBadge";
import { createMemo, For, Show, type JSX } from "solid-js";
import { accountSharers, type AtlasAgent, type AtlasContainer } from "./atlas-graph";
import type { AtlasWork } from "./atlas-work";

/** A container's state and since when, as "running since 14:02". */
export function containerLine(container: AtlasContainer): string {
    return `${container.state} since ${clockTime(container.sinceMs)}`;
}

export function CardFacts(props: { agent: AtlasAgent; agents: ReadonlyMap<string, AtlasAgent>; work: AtlasWork | null }): JSX.Element {
    const sharers = createMemo(() => accountSharers(props.agent, props.agents.values()));
    return (
        <>
            <Show when={props.agent.runtime}>
                {(runtime) => (
                    <>
                        <dt>Runtime</dt>
                        <dd data-testid="atlas-card-runtime">
                            <RuntimeBadge runtime={runtime()} size="tag" />
                        </dd>
                    </>
                )}
            </Show>
            <Show when={props.agent.container}>
                {(container) => (
                    <>
                        <dt>Container</dt>
                        <dd data-testid="atlas-card-container">
                            {container().name}
                            <span class="atlas-card__note">{containerLine(container())}</span>
                            <span class="atlas-card__note">{container().image}</span>
                        </dd>
                    </>
                )}
            </Show>
            <Show when={props.agent.account}>
                {(account) => (
                    <>
                        <dt>Account</dt>
                        <dd data-testid="atlas-card-account">
                            {account().label ?? "Not signed in on this computer"}
                            <Show when={sharers().length > 0}>
                                <span class="atlas-card__note">Also on it: {sharers().join(", ")}</span>
                            </Show>
                        </dd>
                    </>
                )}
            </Show>
            <Show when={props.work}>{(work) => <WorkRows work={work()} />}</Show>
        </>
    );
}

function WorkRows(props: { work: AtlasWork }): JSX.Element {
    return (
        <>
            <Show when={props.work.branch || props.work.repo}>
                <dt>{props.work.branch ? "Branch" : "Repository"}</dt>
                <dd data-testid="atlas-card-branch">{[props.work.repo, props.work.branch].filter(Boolean).join(" · ")}</dd>
            </Show>
            <Show when={props.work.tasks.length > 0}>
                <dt>Tasks</dt>
                <dd>
                    <ul class="atlas-card__tasks" data-testid="atlas-card-tasks">
                        <For each={props.work.tasks}>
                            {(task) => (
                                <li>
                                    {/* Two lines at most: the whole title is its hover label. */}
                                    <span class="atlas-card__task" {...tip(task.title)}>
                                        #{task.num} {task.title}
                                    </span>
                                    <Show when={task.pr}>
                                        {(pr) => (
                                            <Button
                                                density="compact"
                                                icon="code-pull-request"
                                                onClick={() => void openLink(pr().url)}
                                            >
                                                {pr().label}
                                            </Button>
                                        )}
                                    </Show>
                                </li>
                            )}
                        </For>
                    </ul>
                </dd>
            </Show>
        </>
    );
}
