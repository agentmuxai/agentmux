// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OriginContainer: everything in the conversation that is not the human's own words, in one
 * container (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §4). The header says what it is
 * from the origin srv signed (`node.origin`), or "CLI message" / "Earlier message" when no origin
 * verified; the text is only ever the body, shown as text. Only the human's turn has the inverted
 * style (`--human-turn-*`, human-turn-tokens.test.ts); nothing here uses it.
 *
 * It renders the `system_note` node: AgentMux's own notes, a message from a service, a schedule,
 * a task or the Supervisor, a turn the CLI wrote, and a record from before origins were recorded;
 * and the `jekt_message` node, its `agent` variant: a message another agent sent, closed by default
 * like a tool call, with the tier and delivery tags, the sender's trust, and the raw payload.
 * Jekt: SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md §3.3; the live-arrival hold:
 * REPORT_JEKT_COLLAPSE_AND_PREVIEW_SKID_2026_10_07.md §2.1.
 */

import { Show, createEffect, createMemo, createSignal, untrack, type JSX } from "solid-js";
import { Button } from "@/app/element/ui/Button";
import { Tag } from "@/app/element/ui/Tag";
import type { MessageOrigin } from "@/types/rpc/MessageOrigin";
import { AttachmentStrip } from "../attachments/AttachmentStrip";
import { proseDoc, rawDoc } from "../preview-text/docs";
import type { JektMessageNode, SystemNoteNode } from "../types";
import { JEKT_DELIVERY_ICONS, JEKT_TIER_ICONS } from "../types";
import { arrivedLive } from "../virtualization/live-arrival";
import { CollapsibleMessage } from "./CollapsibleMessage";
import { PreviewLines } from "./PreviewLines";
import { previewBox } from "./scroll-handoff";

/** srv marks a message delivered more than two minutes after it was sent (`DELAYED_AFTER_MS`). */
const DELAYED_AFTER_MS = 120_000;

/** A body this long folds to a few lines until opened. */
const FOLD_CHARS = 280;

export type ContainerKind = MessageOrigin["kind"] | "cli" | "earlier";

interface Variant {
    kind: ContainerKind;
    label: string;
    icon: string;
}

/** The variant a note shows as: its signed origin's, else what the parser knows about the record. */
export function variantOf(node: SystemNoteNode): Variant {
    const origin = node.origin;
    const from = origin?.from;
    if (!origin) {
        if (node.source === "cli") return { kind: "cli", label: "CLI message", icon: "terminal" };
        if (node.source === "earlier") return { kind: "earlier", label: "Earlier message", icon: "clock-rotate-left" };
        return { kind: "system", label: "AgentMux", icon: "circle-info" };
    }
    switch (origin.kind) {
        case "agent":
            return { kind: "agent", label: from ? `Message from ${from}` : "Message from an agent", icon: "paper-plane" };
        case "service":
            return { kind: "service", label: from ? `Notice from ${from}` : "Notice", icon: "bell" };
        case "schedule":
            return { kind: "schedule", label: from ? `Scheduled: ${from}` : "Scheduled", icon: "clock" };
        case "task":
            return { kind: "task", label: "Task notice", icon: "list-check" };
        case "supervisor":
            return { kind: "supervisor", label: "Supervisor", icon: "eye" };
        default:
            return { kind: origin.kind, label: "AgentMux", icon: "circle-info" };
    }
}

/** "11 min late" when the typed times say the message waited, else nothing. */
export function lateBy(origin: MessageOrigin | undefined): string | undefined {
    if (!origin?.sent_ms || !origin.delivered_ms) return undefined;
    const ms = origin.delivered_ms - origin.sent_ms;
    if (ms <= DELAYED_AFTER_MS) return undefined;
    const min = Math.round(ms / 60_000);
    return min < 60 ? `${min} min late` : `${Math.floor(min / 60)} h ${min % 60} min late`;
}

export interface OriginContainerProps {
    node: SystemNoteNode | JektMessageNode;
    /** The jekt variant opens and closes like a tool call. */
    collapsed?: boolean;
    onToggle?: () => void;
    /** Hold the jekt row open: called once when it arrives live. */
    onHoldOpen?: () => void;
}

export function OriginContainer(props: OriginContainerProps): JSX.Element {
    return (
        <Show when={props.node.type === "jekt_message"} fallback={<NoteVariant node={props.node as SystemNoteNode} />}>
            <JektVariant
                node={props.node as JektMessageNode}
                collapsed={props.collapsed ?? true}
                onToggle={props.onToggle ?? (() => {})}
                onHoldOpen={props.onHoldOpen}
            />
        </Show>
    );
}

function NoteVariant(props: { node: SystemNoteNode }): JSX.Element {
    const variant = () => variantOf(props.node);
    const late = () => props.node.delay ?? lateBy(props.node.origin);
    const [open, setOpen] = createSignal(false);
    // Only a body that can be long folds; a short note is one line. The startup payload srv signs as a
    // system message is the long case for a note.
    const long = () => props.node.text.length > FOLD_CHARS || props.node.text.split("\n").length > 4;
    const folded = () => long() && !open();
    return (
        <div
            class="agent-origin agent-origin--flat"
            classList={{ [`agent-origin--${variant().kind}`]: true, "agent-origin--folded": folded() }}
            data-origin={variant().kind}
        >
            <span class="agent-origin__label">
                <i class={`fa-solid fa-${variant().icon}`} aria-hidden="true" /> {variant().label}
            </span>
            <Show when={props.node.detail}>{(detail) => <span class="agent-origin__detail">{detail()}</span>}</Show>
            {/* Colour marks a trust problem only: a sender nothing proved, a signature that did not hold. */}
            <Show when={props.node.origin?.trust}>
                {(trust) => (
                    <Tag tone={trust().endsWith("-verified") ? "neutral" : "warn"} detail={`Trust: ${trust()}`}>
                        {trust()}
                    </Tag>
                )}
            </Show>
            <Show when={props.node.origin?.sig}>
                {(sig) => (
                    <Tag tone={sig() === "verified" ? "neutral" : "warn"} detail={`Signature: ${sig()}`}>
                        {sig() === "verified" ? "signed" : "signature invalid"}
                    </Tag>
                )}
            </Show>
            <Show when={late()}>{(text) => <span class="agent-origin__delay">{text()}</span>}</Show>
            <Show when={props.node.attachments?.length}>
                <AttachmentStrip attachments={props.node.attachments!} />
            </Show>
            {/* Text only: a message is never read as markup. */}
            <span class="agent-origin__text">{props.node.text}</span>
            <Show when={long()}>
                <Button variant="link" class="agent-origin__more" onClick={() => setOpen(!open())}>
                    {open() ? "Show less" : "Show more"}
                </Button>
            </Show>
        </div>
    );
}

function formatTimestamp(ts: number): string {
    return new Date(ts).toLocaleString();
}

/** "45 s", "12 min", "3 h 5 min": how long a held jekt waited. */
function formatHeldFor(secs: number): string {
    if (secs < 60) return `${secs} s`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins} min`;
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m ? `${h} h ${m} min` : `${h} h`;
}

/** When the jekt reached this pane: its send time, plus how long it waited if it was held for an
 *  absent recipient (`HELD_FOR`). A held jekt delivered just now arrived live, however long ago it was sent. */
function deliveredAt(node: Pick<JektMessageNode, "timestamp" | "heldForSecs">): number {
    return node.timestamp + (node.heldForSecs ?? 0) * 1000;
}

/** Both capped boxes (body, raw payload) skid and hand scroll to the pane at their edges, like a tool preview. */
const handoff = previewBox();

/** Nothing proved a jekt's sender (anything but a `*-verified` trust): shown as a warning. */
const unproven = (trust: string): boolean => !trust.endsWith("-verified");

function JektVariant(props: {
    node: JektMessageNode;
    collapsed: boolean;
    onToggle: () => void;
    onHoldOpen?: () => void;
}): JSX.Element {
    // Built once per message, not on every read of the prop.
    const bodyDoc = createMemo(() => proseDoc(props.node.message));
    // Exactly as received: no terminal decoding, control characters visible.
    const rawPayload = createMemo(() => rawDoc(props.node.raw));
    // A jekt that arrives live is held open until it scrolls off, as a tool that finishes on screen is.
    // Keyed on the id: the streaming buffer can reuse this row for another node. A sensitive jekt is open by default anyway.
    let checkedId: string | undefined;
    createEffect(() => {
        const node = props.node;
        if (node.id === checkedId) return;
        checkedId = node.id;
        if (node.tier !== "sensitive" && arrivedLive(deliveredAt(node))) untrack(() => props.onHoldOpen?.());
    });
    const incoming = () => props.node.direction === "incoming";

    return (
        // Don't destructure props: see CollapsibleMessage. The row, toggle, chevron and peek come from there.
        <CollapsibleMessage
            rootClass="agent-origin agent-origin--agent"
            classPrefix="agent-origin"
            classes={{ incoming: incoming(), outgoing: !incoming(), [`tier-${props.node.tier}`]: true }}
            collapsed={props.collapsed}
            onToggle={props.onToggle}
            peekText={props.node.message}
            timestamp={props.node.timestamp}
            summary={
                <>
                    <span class="agent-origin__label agent-origin__peer">
                        <i class={`fa-solid fa-${incoming() ? "arrow-down-long" : "arrow-up-long"}`} aria-hidden="true" />{" "}
                        {incoming() ? `Jekt from ${props.node.from}` : `Jekt to ${props.node.to}`}
                    </span>
                    <Tag tone={props.node.tier === "sensitive" ? "error" : "neutral"} detail={`Tier: ${props.node.tier}`}>
                        {JEKT_TIER_ICONS[props.node.tier]} {props.node.tier}
                    </Tag>
                    <Tag tone={unproven(props.node.trust) ? "warn" : "neutral"} detail={`Delivery: ${props.node.deliveryTier} (${props.node.trust})`}>
                        {JEKT_DELIVERY_ICONS[props.node.deliveryTier]} {props.node.deliveryTier}
                    </Tag>
                </>
            }
            body={
                <>
                    {/* Files sent with it, above the text as on a user message. */}
                    <Show when={props.node.attachments?.length}>
                        <AttachmentStrip attachments={props.node.attachments!} />
                    </Show>
                    {/* Prose: wraps at words (PreviewLines, the preview text stage). */}
                    <div class="agent-origin-body" ref={handoff}>
                        <PreviewLines doc={bodyDoc()} linkify />
                    </div>
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
                    <details class="agent-origin-raw">
                        <summary>Raw payload</summary>
                        <div class="agent-origin-raw-body" ref={handoff}>
                            <PreviewLines doc={rawPayload()} />
                        </div>
                    </details>
                </>
            }
        />
    );
}

OriginContainer.displayName = "OriginContainer";
