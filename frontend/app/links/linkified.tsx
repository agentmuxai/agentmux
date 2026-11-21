// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Plain text with the entities in it made links (spec §4.5): the one renderer the plain-text surfaces share. The
 * segments come from `linkify`; a click opens through the entity's kind and never navigates the window. A link sits
 * inside cards that are themselves pressable, so its press and its keys stop at the link.
 */

import { createMemo, For, type JSX } from "solid-js";
import { onLinkAuxClick, onLinkClick } from "@/app/element/link-open";
import { tip } from "@/app/element/tip";
import { followAgentNames } from "@/app/store/agent-names";
import { pushNotification } from "@/app/store/global";
import { takeFloatRequest } from "@/app/store/float-modifier";
import { linkify } from "./entities/linkify";
import { openEntity } from "./entities/registry";
import type { Segment } from "./entities/types";
import { hrefOf, webUrlOf } from "./entity-href";
import { linkContext, type SurfaceContext } from "./link-context";
import "./linkified.scss";

type LinkSegment = Extract<Segment, { type: "link" }>;

function press(seg: LinkSegment, e: MouseEvent): void {
    e.stopPropagation();
    const web = webUrlOf(seg.destination);
    // A web page opens the way every web link does: the person's browser, a browser pane on Ctrl or Cmd.
    if (web) return onLinkClick(e, web);
    e.preventDefault();
    void openEntity(seg.destination, { floating: takeFloatRequest() }).then((r) => {
        if (!("reason" in r)) return;
        pushNotification({
            icon: "fa-triangle-exclamation",
            title: `Couldn't open ${seg.label}`,
            message: r.reason,
            timestamp: new Date().toISOString(),
            type: "error",
            expiration: Date.now() + 8000,
        });
    });
}

function EntityLink(props: { seg: LinkSegment }): JSX.Element {
    const seg = () => props.seg;
    return (
        <a
            href={hrefOf(seg().destination)}
            class={`entity-link entity-link--${seg().kind}`}
            classList={{ "entity-link--dead": seg().state === "dead" }}
            aria-label={seg().name}
            {...tip(seg().name)}
            onClick={(e) => press(seg(), e)}
            onAuxClick={(e) => {
                e.stopPropagation();
                const web = webUrlOf(seg().destination);
                if (web) onLinkAuxClick(e, web);
                else if (e.button === 1) e.preventDefault();
            }}
            onKeyDown={(e) => {
                // The card around it opens on Enter and Space; on a link they are the link's.
                if (e.key === "Enter" || e.key === " ") e.stopPropagation();
            }}
        >
            {seg().label}
        </a>
    );
}

/** An agent already known (a card's holder): a link to its pane, with nothing to recognise. */
export function AgentLink(props: { id: string; name: string }): JSX.Element {
    const seg = (): LinkSegment => ({
        type: "link",
        kind: "agent",
        label: props.name,
        destination: { kind: "agent", id: props.id, name: props.name },
        state: "ready",
        name: `Open agent ${props.name}`,
    });
    return <EntityLink seg={seg()} />;
}

/** Text with its agent names, task numbers, pull requests and web addresses as links. */
export function Linkified(props: { text: string; surface?: SurfaceContext }): JSX.Element {
    followAgentNames();
    const segments = createMemo(() => linkify(props.text, linkContext(props.surface)));
    return (
        <For each={segments()}>{(seg) => (seg.type === "link" ? <EntityLink seg={seg} /> : <>{seg.text}</>)}</For>
    );
}
