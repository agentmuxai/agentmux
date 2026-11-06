// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The parts of a jekt that are the same wherever it is shown: the transcript's open row (OriginContainer) and the
 * Messages panel (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md §5.2). One place renders the
 * tags and the metadata row, so the two cannot disagree about a message's tier, delivery or trust.
 */

import { Tag } from "@/app/element/ui/Tag";
import { Show, type JSX } from "solid-js";
import type { JektMessageNode } from "../../../agent-model/types";
import { JEKT_DELIVERY_ICONS, JEKT_TIER_ICONS } from "../../../agent-model/types";

/** Nothing proved a jekt's sender (anything but a `*-verified` trust): shown as a warning. */
export const unproven = (trust: string): boolean => !trust.endsWith("-verified");

export function formatTimestamp(ts: number): string {
    return new Date(ts).toLocaleString();
}

/** "45 s", "12 min", "3 h 5 min": how long a held jekt waited. */
export function formatHeldFor(secs: number): string {
    if (secs < 60) return `${secs} s`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins} min`;
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m ? `${h} h ${m} min` : `${h} h`;
}

/** The tier tag and the delivery tag (whose tone says whether the sender was proven). */
export function JektTags(props: { node: JektMessageNode }): JSX.Element {
    return (
        <>
            <Tag tone={props.node.tier === "sensitive" ? "error" : "neutral"} detail={`Tier: ${props.node.tier}`}>
                {JEKT_TIER_ICONS[props.node.tier]} {props.node.tier}
            </Tag>
            <Tag
                tone={unproven(props.node.trust) ? "warn" : "neutral"}
                detail={`Delivery: ${props.node.deliveryTier} (${props.node.trust})`}
            >
                {JEKT_DELIVERY_ICONS[props.node.deliveryTier]} {props.node.deliveryTier}
            </Tag>
        </>
    );
}

/** From, To, MSGID, trust, priority, time, and how long it was held. */
export function JektMeta(props: { node: JektMessageNode }): JSX.Element {
    return (
        <div class="agent-origin-meta">
            <span class="agent-origin-meta-item">From: {props.node.from}</span>
            <span class="agent-origin-meta-item">To: {props.node.to}</span>
            <span class="agent-origin-meta-item">MSGID: {props.node.msgId || "—"}</span>
            <span class="agent-origin-meta-item">Trust: {props.node.trust}</span>
            <span class="agent-origin-meta-item">Priority: {props.node.priority}</span>
            <span class="agent-origin-meta-item">{formatTimestamp(props.node.timestamp)}</span>
            <Show when={props.node.heldForSecs !== undefined}>
                <Tag detail="The recipient was not running; this was held and delivered when it started">
                    Held for {formatHeldFor(props.node.heldForSecs ?? 0)}
                </Tag>
            </Show>
        </div>
    );
}
