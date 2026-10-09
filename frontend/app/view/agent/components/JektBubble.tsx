// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * JektBubble — agent-pane rendering of a muxbus jekt message.
 *
 * Replaces the raw `[JEKT:FROM=... TIER=... ...]` marker text with a
 * distinct, labeled block so the human operator can always tell a jekt
 * apart from a typed user message or agent output (design goals G1/G2).
 *
 * Closed by default, like a tool call: a one-line summary showing
 * direction, sender/recipient, and the tier + delivery badges. A jekt that
 * arrives live is held open until it scrolls off the top, a click pins it
 * open, and a sensitive one starts open (virtualization/disclosure.ts,
 * REPORT_JEKT_COLLAPSE_AND_PREVIEW_SKID_2026_10_07.md §2.1). Open, it shows
 * the message body plus metadata (full MSGID, timestamp, raw marker payload)
 * — spec §3.3's "click the bubble shows metadata".
 *
 * Spec: docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md §3.3.
 */

import { Show, createEffect, untrack, type JSX } from "solid-js";
import { outputDoc, proseDoc } from "../preview-text/docs";
import type { JektMessageNode } from "../types";
import { JEKT_DELIVERY_ICONS, JEKT_TIER_ICONS } from "../types";
import { arrivedLive } from "../virtualization/live-arrival";
import { CollapsibleMessage } from "./CollapsibleMessage";
import { PreviewLines } from "./PreviewLines";
import { previewBox } from "./scroll-handoff";

interface JektBubbleProps {
    node: JektMessageNode;
    collapsed: boolean;
    onToggle: () => void;
    /** Hold the row open: called once when the jekt arrives live. */
    onHoldOpen?: () => void;
}

function formatTimestamp(ts: number): string {
    return new Date(ts).toLocaleString();
}

/** "45 s", "12 min", "3 h 5 min" — how long a held jekt waited. */
function formatHeldFor(secs: number): string {
    if (secs < 60) return `${secs} s`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins} min`;
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m ? `${h} h ${m} min` : `${h} h`;
}

/** When the jekt reached this pane: its send time, plus how long it waited if
 *  it was held for an absent recipient (`HELD_FOR`). A held jekt delivered
 *  just now arrived live, however long ago it was sent. */
export function deliveredAt(node: Pick<JektMessageNode, "timestamp" | "heldForSecs">): number {
    return node.timestamp + (node.heldForSecs ?? 0) * 1000;
}

/** Both capped boxes (body, raw payload) skid and hand scroll to the pane at
 *  their edges, like a tool preview (scroll-handoff.ts `previewBox`). */
const handoff = previewBox();

export const JektBubble = (props: JektBubbleProps): JSX.Element => {
    // A jekt that arrives live is held open until it scrolls off, as a tool that
    // finishes on screen is. Keyed on the id: the streaming buffer can reuse this
    // row for another node. A sensitive jekt is open by default anyway.
    let checkedId: string | undefined;
    createEffect(() => {
        const node = props.node;
        if (node.id === checkedId) return;
        checkedId = node.id;
        if (node.tier !== "sensitive" && arrivedLive(deliveredAt(node))) untrack(() => props.onHoldOpen?.());
    });

    return (
        // Don't destructure props — see CollapsibleMessage for why. The row,
        // toggle, chevron and peek come from there; the peek adds a relative
        // time, which the expanded view's toLocaleString() line lacks.
        <CollapsibleMessage
            rootClass="agent-jekt-bubble"
            classPrefix="agent-jekt"
            classes={{
                incoming: props.node.direction === "incoming",
                outgoing: props.node.direction === "outgoing",
                [`tier-${props.node.tier}`]: true,
            }}
            collapsed={props.collapsed}
            onToggle={props.onToggle}
            peekText={props.node.message}
            timestamp={props.node.timestamp}
            summary={
                <>
                    <span class="agent-jekt-direction-icon">{props.node.direction === "incoming" ? "📥" : "📤"}</span>
                    <span class="agent-jekt-peer">
                        {props.node.direction === "incoming" ? `From ${props.node.from}` : `To ${props.node.to}`}
                    </span>
                    <span class="agent-jekt-tier-badge" title={`Tier: ${props.node.tier}`}>
                        {JEKT_TIER_ICONS[props.node.tier]} {props.node.tier}
                    </span>
                    <span
                        class="agent-jekt-delivery-badge"
                        title={`Delivery: ${props.node.deliveryTier} (${props.node.trust})`}
                    >
                        {JEKT_DELIVERY_ICONS[props.node.deliveryTier]} {props.node.deliveryTier}
                    </span>
                </>
            }
            body={
                <>
                    {/* Prose: wraps at words (PreviewLines, the preview text stage). */}
                    <div class="agent-jekt-body" ref={handoff}>
                        <PreviewLines doc={proseDoc(props.node.message)} linkify />
                    </div>
                    <div class="agent-jekt-meta">
                        <span class="agent-jekt-meta-item">From: {props.node.from}</span>
                        <span class="agent-jekt-meta-item">To: {props.node.to}</span>
                        <span class="agent-jekt-meta-item">MSGID: {props.node.msgId || "—"}</span>
                        <span class="agent-jekt-meta-item">Trust: {props.node.trust}</span>
                        <span class="agent-jekt-meta-item">Priority: {props.node.priority}</span>
                        <span class="agent-jekt-meta-item">{formatTimestamp(props.node.timestamp)}</span>
                        <Show when={props.node.heldForSecs !== undefined}>
                            <span
                                class="agent-jekt-meta-item"
                                title="The recipient was not running; this was held and delivered when it started"
                            >
                                Held for {formatHeldFor(props.node.heldForSecs ?? 0)}
                            </span>
                        </Show>
                    </div>
                    <details class="agent-jekt-raw">
                        <summary>Raw payload</summary>
                        <div class="agent-jekt-raw-body" ref={handoff}>
                            <PreviewLines doc={outputDoc(props.node.raw, { from: "head" })} />
                        </div>
                    </details>
                </>
            }
        />
    );
};

JektBubble.displayName = "JektBubble";
