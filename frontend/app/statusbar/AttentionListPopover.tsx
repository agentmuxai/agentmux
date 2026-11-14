// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The list the status bar's agent entry opens (SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02 §3.6): one row per
 * pane that waits on the person, the oldest first: the agent's name in its colour, what it wants, how long, and a
 * button that takes them there ("Go", or "Open" for an agent that runs with no pane). Under them, when the Kanban
 * has tasks that need the person, one row for those that opens the Kanban (they make no sound, so no toast).
 */

import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { Button } from "@/app/element/ui";
import { useTick } from "@/app/hook/useTick";
import { goToAttention, isParkedItem } from "@/app/notification/attention-go";
import { attentionTint, type AttentionItem } from "@/app/notification/attention-list";
import { createMemo, For, Show, type Accessor, type JSX } from "solid-js";
import { waitedFor } from "./waited-for";

export interface AttentionListPopoverProps {
    anchor: PopoverAnchor;
    items: Accessor<AttentionItem[]>;
    /** How many Kanban tasks need the person; none when absent. */
    tasks?: Accessor<number>;
    onClose: () => void;
    onOpenSwarm: () => void;
    onOpenKanban?: () => void;
    /** What a row's button runs; default takes the person to the pane. */
    go?: (item: AttentionItem) => void;
}

export const AttentionListPopover = (props: AttentionListPopoverProps): JSX.Element => {
    const tick = useTick(10_000);
    const now = createMemo(() => (tick(), Date.now()));
    const tasks = () => props.tasks?.() ?? 0;
    const go = (item: AttentionItem) => {
        props.onClose();
        (props.go ?? ((i) => void goToAttention(i)))(item);
    };
    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            onDismiss={props.onClose}
            class="attention-list"
            role="dialog"
            aria-label="Waiting on you"
            style={{ width: "360px" }}
        >
            <div class="attention-list-header">
                <span class="attention-list-title">Waiting on you</span>
                <span class="attention-list-count">{props.items().length + tasks()}</span>
            </div>
            <Show
                when={props.items().length > 0 || tasks() > 0}
                fallback={<div class="attention-list-empty">Nothing waits on you.</div>}
            >
                <ul class="attention-list-rows">
                    <For each={props.items()}>
                        {(item) => (
                            <li class="attention-list-row">
                                <div class="attention-list-text">
                                    <span class="attention-list-name" style={{ color: attentionTint(item.color) }}>
                                        {item.name}
                                    </span>
                                    <span class="attention-list-what">
                                        {item.what}
                                        {item.local ? "" : " (in another window)"}
                                    </span>
                                </div>
                                <span class="attention-list-age">{waitedFor(item.since, now())}</span>
                                <Button
                                    density="compact"
                                    aria-label={`${isParkedItem(item) ? "Open" : "Go to"} ${item.name}`}
                                    onClick={() => go(item)}
                                >
                                    {isParkedItem(item) ? "Open" : "Go"}
                                </Button>
                            </li>
                        )}
                    </For>
                    <Show when={tasks() > 0}>
                        <li class="attention-list-row attention-list-row--kanban">
                            <div class="attention-list-text">
                                <span class="attention-list-name">Kanban</span>
                                <span class="attention-list-what">
                                    {tasks() === 1 ? "1 task needs you" : `${tasks()} tasks need you`}
                                </span>
                            </div>
                            <Button
                                density="compact"
                                aria-label="Open the Kanban"
                                onClick={() => {
                                    props.onClose();
                                    props.onOpenKanban?.();
                                }}
                            >
                                Open
                            </Button>
                        </li>
                    </Show>
                </ul>
            </Show>
            <div class="attention-list-footer">
                <Button
                    density="compact"
                    onClick={() => {
                        props.onClose();
                        props.onOpenSwarm();
                    }}
                >
                    Open the Swarm
                </Button>
            </div>
        </AnchoredPopover>
    );
};

AttentionListPopover.displayName = "AttentionListPopover";
