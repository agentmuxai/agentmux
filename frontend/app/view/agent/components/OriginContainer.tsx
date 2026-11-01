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
 * a task or the Supervisor, a turn the CLI wrote, and a record from before origins were recorded.
 * A jekt from an agent keeps `JektBubble` until it becomes this container's `agent` variant.
 */

import { Show, createSignal, type JSX } from "solid-js";
import { Button } from "@/app/element/ui/Button";
import { Tag } from "@/app/element/ui/Tag";
import type { MessageOrigin } from "@/types/rpc/MessageOrigin";
import type { SystemNoteNode } from "../types";

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

export function OriginContainer(props: { node: SystemNoteNode }): JSX.Element {
    const variant = () => variantOf(props.node);
    const late = () => props.node.delay ?? lateBy(props.node.origin);
    const [open, setOpen] = createSignal(false);
    // Only a body that can be long folds; a short note is one line.
    const long = () => props.node.text.length > FOLD_CHARS || props.node.text.split("\n").length > 4;
    const folded = () => long() && !open() && variant().kind !== "system";
    return (
        <div
            class="agent-origin"
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
            {/* Text only: a message is never read as markup. */}
            <span class="agent-origin__text">{props.node.text}</span>
            <Show when={long() && variant().kind !== "system"}>
                <Button variant="link" class="agent-origin__more" onClick={() => setOpen(!open())}>
                    {open() ? "Show less" : "Show more"}
                </Button>
            </Show>
        </div>
    );
}

OriginContainer.displayName = "OriginContainer";
