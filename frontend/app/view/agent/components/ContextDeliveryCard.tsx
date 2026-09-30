// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ContextDeliveryCard — what the agent was given without the user typing it,
 * as one card with a row per item. Collapsed, it's a title, a size and a
 * one-line excerpt; pinned open, each item shows its name, size and full text.
 *
 * The first kind is Claude Code's compaction summary (CD1), which used to
 * render as a 20 KB user message. docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.2.
 *
 * Don't destructure props — see CollapsibleMessage.
 */

import { For, Show, onCleanup, type JSX } from "solid-js";
import { LinkifiedText } from "@/app/element/linkified-text";
import { formatCompactNumber } from "@/util/format-count";
import { contextDeliveryTitle } from "../context-delivery";
import type { ContextDeliveryNode, ContextItem } from "../types";
import { CollapsibleMessage } from "./CollapsibleMessage";
import { attachScrollHandoff } from "./scroll-handoff";

interface ContextDeliveryCardProps {
    node: ContextDeliveryNode;
    pinned: boolean;
    onTogglePin: () => void;
}

const ITEM_ICON: Record<ContextItem["kind"], string> = {
    global_memory: "🌐",
    personal_memory: "👤",
    running_summary: "🧭",
    compaction_summary: "📋",
    continuation_packet: "🔁",
};

/** A long body scrolls inside its own box and hands scroll to the pane at its edges. */
const handoff = (el: HTMLElement): void => {
    onCleanup(attachScrollHandoff(el));
};

const totalTokens = (node: ContextDeliveryNode): number => node.items.reduce((sum, i) => sum + i.tokens, 0);

export const ContextDeliveryCard = (props: ContextDeliveryCardProps): JSX.Element => (
    <CollapsibleMessage
        rootClass="agent-context-delivery"
        classPrefix="agent-context-delivery"
        classes={{ [`reason-${props.node.reason}`]: true }}
        collapsed={!props.pinned}
        onToggle={props.onTogglePin}
        peekText={props.node.items.map((i) => i.body ?? "").join("\n")}
        timestamp={props.node.timestamp}
        summary={
            <>
                <span class="agent-context-delivery-icon">
                    {props.node.items.length === 1 ? ITEM_ICON[props.node.items[0].kind] : "📥"}
                </span>
                <span class="agent-context-delivery-title">{contextDeliveryTitle(props.node)}</span>
                <span class="agent-context-delivery-size">~{formatCompactNumber(totalTokens(props.node))} tok (est.)</span>
                <Show when={!props.pinned && props.node.items[0]?.excerpt}>
                    <span class="agent-context-delivery-excerpt">{props.node.items[0].excerpt}</span>
                </Show>
            </>
        }
        body={
            <For each={props.node.items}>
                {(item) => (
                    <div class="agent-context-delivery-item">
                        <div class="agent-context-delivery-item-head">
                            <span class="agent-context-delivery-item-icon">{ITEM_ICON[item.kind]}</span>
                            <span class="agent-context-delivery-item-name">{item.name}</span>
                            <span class="agent-context-delivery-item-size">
                                {formatCompactNumber(item.sizeBytes)} B · ~{formatCompactNumber(item.tokens)} tok (est.)
                            </span>
                        </div>
                        <Show when={item.body}>
                            <pre class="agent-context-delivery-body" ref={handoff}>
                                <LinkifiedText text={item.body ?? ""} />
                            </pre>
                        </Show>
                    </div>
                )}
            </For>
        }
    />
);

ContextDeliveryCard.displayName = "ContextDeliveryCard";
