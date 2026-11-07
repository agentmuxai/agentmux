// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The list the status bar's agent entry opens (SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02 §3.6): one row per
 * pane that waits on the person, the oldest first: the agent's name in its colour, what it wants, how long, and a
 * button that takes them there ("Go", or "Open" for an agent that runs with no pane).
 */

import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { Button } from "@/app/element/ui";
import { useTick } from "@/app/hook/useTick";
import { formatLocale } from "@/app/i18n/i18n";
import { goToAttention, isParkedItem } from "@/app/notification/attention-go";
import { attentionTint, type AttentionItem } from "@/app/notification/attention-list";
import { formatRelativeTime } from "@/util/format-time";
import { createMemo, For, Show, type Accessor, type JSX } from "solid-js";

export interface AttentionListPopoverProps {
    anchor: PopoverAnchor;
    items: Accessor<AttentionItem[]>;
    onClose: () => void;
    onOpenSwarm: () => void;
    /** What a row's button runs; default takes the person to the pane. */
    go?: (item: AttentionItem) => void;
}

/** How long a row has waited, "now" below a minute. */
export function waitedFor(since: number, now: number): string {
    return formatRelativeTime(since, now, { locale: formatLocale(), nowBelowSecs: 60 });
}

export const AttentionListPopover = (props: AttentionListPopoverProps): JSX.Element => {
    const tick = useTick(10_000);
    const now = createMemo(() => (tick(), Date.now()));
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
            aria-label="Agents waiting on you"
            style={{ width: "360px" }}
        >
            <div class="attention-list-header">
                <span class="attention-list-title">Waiting on you</span>
                <span class="attention-list-count">{props.items().length}</span>
            </div>
            <Show when={props.items().length > 0} fallback={<div class="attention-list-empty">Nothing waits on you.</div>}>
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
