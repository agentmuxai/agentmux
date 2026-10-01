// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ContextDeliveryCard — what the agent was given without the user typing it,
 * as one card with a row per item.
 *
 * - Claude Code's compaction summary (CD1): a title, a size and a one-line
 *   excerpt; pinned open, the full summary.
 * - Memory deliveries (CD2a): a row per item, always visible: kind, name,
 *   tier (AgentMux system, workspace, personal), file, size, and a "cut" mark
 *   when the part cap cut it.
 *
 * docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.2. Don't destructure
 * props — see CollapsibleMessage.
 */

import clsx from "clsx";
import { For, Show, onCleanup, type JSX } from "solid-js";
import { LinkifiedText } from "@/app/element/linkified-text";
import { formatCompactNumber } from "@/util/format-count";
import { contextDeliveryTitle, isCompactionSummaryCard } from "../context-delivery";
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
    startup_file: "📄",
};

const OWNER_CHIP: Record<NonNullable<ContextItem["owner"]>, { text: string; title: string }> = {
    agentmux: { text: "AgentMux", title: "Written by AgentMux; regenerated at launch" },
    user: { text: "Yours", title: "In the agent's workspace, not managed by AgentMux" },
    external: { text: "Hand-maintained", title: "Outside the workspace and not written by AgentMux" },
};

/** The chip after an item's name, or null. */
function chip(item: ContextItem): string | null {
    if (item.tier === "system") return "AgentMux system";
    if (item.tier === "workspace") return "Workspace";
    if (item.kind === "personal_memory") return "Personal";
    if (item.owner) return OWNER_CHIP[item.owner].text;
    return null;
}

/** What the size column says: a count for the skill and MCP listings, which carry no file text. */
function sizeText(item: ContextItem): string {
    if (item.count != null) return `${item.count} listed`;
    if (item.via === "startup_file") return "in startup file";
    return `${fmt(item.sizeBytes)} B · ~${fmt(item.tokens)} tok (est.)`;
}

/** A long body scrolls inside its own box and hands scroll to the pane at its edges. */
const handoff = (el: HTMLElement): void => {
    onCleanup(attachScrollHandoff(el));
};

const totalTokens = (node: ContextDeliveryNode): number => node.items.reduce((sum, i) => sum + i.tokens, 0);

const fmt = formatCompactNumber;

/** One item's head row: icon, name, chip, cut mark, size. The file shows on hover. */
const ItemHead = (props: { item: ContextItem }): JSX.Element => (
    <div class={clsx("agent-context-delivery-item-head", props.item.delivered && `delivered-${props.item.delivered}`)}>
        <span class="agent-context-delivery-item-icon">{ITEM_ICON[props.item.kind]}</span>
        <span class="agent-context-delivery-item-name" title={props.item.path ?? props.item.name}>
            {props.item.name}
        </span>
        <Show when={chip(props.item)}>
            <span
                class="agent-context-delivery-item-chip"
                title={props.item.owner ? OWNER_CHIP[props.item.owner].title : undefined}
            >
                {chip(props.item)}
            </span>
        </Show>
        <Show when={props.item.contains?.includes("global_memory")}>
            <span
                class="agent-context-delivery-item-chip agent-context-delivery-item-dup"
                title="This file also carries the Global Memory, so a new session gets it twice"
            >
                + Global Memory
            </span>
        </Show>
        <Show when={props.item.delivered === "partial" || props.item.delivered === "omitted"}>
            <span
                class="agent-context-delivery-item-cut"
                title={
                    props.item.delivered === "omitted"
                        ? "Not delivered: the memory was larger than one delivery can carry"
                        : "Cut: only part of this fit in the delivery"
                }
            >
                {props.item.delivered === "omitted" ? "not sent" : "cut"}
            </span>
        </Show>
        <span class="agent-context-delivery-item-size">{sizeText(props.item)}</span>
    </div>
);

export const ContextDeliveryCard = (props: ContextDeliveryCardProps): JSX.Element => {
    const summaryCard = (): boolean => isCompactionSummaryCard(props.node);
    const bodies = (): ContextItem[] => props.node.items.filter((i) => i.body);
    const bigMemory = (): boolean => props.node.sizeBand === "high" || props.node.sizeBand === "critical";
    const classes = (): Record<string, boolean> => ({
        [`reason-${props.node.reason}`]: true,
        [`size-${props.node.sizeBand}`]: !!props.node.sizeBand,
    });
    const summary = (): JSX.Element => (
        <>
            <span class="agent-context-delivery-icon">{summaryCard() ? ITEM_ICON.compaction_summary : "📥"}</span>
            <span class="agent-context-delivery-title">{contextDeliveryTitle(props.node)}</span>
            <span class="agent-context-delivery-size">~{fmt(totalTokens(props.node))} tok (est.)</span>
            <Show when={summaryCard() && !props.pinned && props.node.items[0]?.excerpt}>
                <span class="agent-context-delivery-excerpt">{props.node.items[0].excerpt}</span>
            </Show>
            {/* Memory deliveries list every item without opening the card. */}
            <Show when={!summaryCard()}>
                <div class="agent-context-delivery-rows">
                    <For each={props.node.items}>{(item) => <ItemHead item={item} />}</For>
                </div>
            </Show>
            <Show when={bigMemory()}>
                <div class="agent-context-delivery-advice">
                    Personal memory has grown large. Consider consolidating it (MemoryList, then MemoryWrite; check
                    MemoryHistory first), or handing the next piece of work to another agent (WorkEnqueue).
                </div>
            </Show>
        </>
    );
    return (
        // Nothing to open (memory items carry no text until CD3): no chevron, no toggle.
        <Show
            when={bodies().length > 0}
            fallback={
                <div class={clsx("agent-context-delivery", "no-body", classes())}>
                    <div class="agent-context-delivery-summary">{summary()}</div>
                </div>
            }
        >
            <CollapsibleMessage
                rootClass="agent-context-delivery"
                classPrefix="agent-context-delivery"
                classes={classes()}
                collapsed={!props.pinned}
                onToggle={props.onTogglePin}
                peekText={props.node.items.map((i) => i.body ?? "").join("\n")}
                timestamp={props.node.timestamp}
                summary={summary()}
                body={
                    <For each={bodies()}>
                        {(item) => (
                            <div class="agent-context-delivery-item">
                                <ItemHead item={item} />
                                <pre class="agent-context-delivery-body" ref={handoff}>
                                    <LinkifiedText text={item.body ?? ""} />
                                </pre>
                            </div>
                        )}
                    </For>
                }
            />
        </Show>
    );
};

ContextDeliveryCard.displayName = "ContextDeliveryCard";
