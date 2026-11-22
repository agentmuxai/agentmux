// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What is on the Atlas map and is not an agent (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.4): a container of
 * this machine that no agent uses, as a node in the machine's region. It opens a card, as an agent's node does; it
 * cannot be selected for a fleet action.
 */

import { CloseButton } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { createMemo, Show, type JSX } from "solid-js";
import { ByKey } from "./atlas-by-key";
import { containerLine } from "./atlas-card-facts";
import type { AtlasGraph, AtlasOrphan } from "./atlas-graph";
import { NODE_H, NODE_W, type Placed } from "./atlas-layout";

type Select = (key: string, e: MouseEvent | KeyboardEvent) => void;

/** A node's box and what makes it a button: a click or Enter or Space opens its card. */
function nodeProps(key: string, at: Placed, selected: boolean, onSelect: Select) {
    return {
        role: "button" as const,
        tabindex: "0",
        style: { left: `${at.x}px`, top: `${at.y}px`, width: `${NODE_W}px`, height: `${NODE_H}px` },
        "aria-pressed": selected,
        onClick: (e: MouseEvent) => onSelect(key, e),
        onKeyDown: (e: KeyboardEvent) => {
            if (e.key !== "Enter" && e.key !== " ") return;
            e.preventDefault();
            onSelect(key, e);
        },
    };
}

function OrphanNode(props: { orphan: AtlasOrphan; at: Placed; selected: boolean; onSelect: Select }): JSX.Element {
    const o = () => props.orphan;
    return (
        <div
            {...nodeProps(o().key, props.at, props.selected, props.onSelect)}
            class="atlas-node atlas-node--orphan"
            classList={{ "atlas-node--selected": props.selected }}
            data-testid="atlas-orphan"
            data-container={o().state}
            aria-label={`${o().name}, ${o().helper ?? "a container no agent uses"}, ${o().state}`}
            {...tip(`${o().helper ?? "A container no agent uses"}, ${containerLine(o())}`)}
        >
            <span class="atlas-node__top">
                <i class="fa-solid fa-box atlas-node__kind" aria-hidden="true" />
                <span class="atlas-node__name">{o().name}</span>
            </span>
            <span class="atlas-node__sub">{o().helper ?? "No agent"} · {o().state}</span>
        </div>
    );
}

/** The nodes that are not agents, where the layout put them. */
export function ExtraNodes(props: {
    graph: AtlasGraph;
    nodes: ReadonlyMap<string, Placed>;
    selected: string | null;
    onSelect: Select;
}): JSX.Element {
    const orphans = createMemo(() => props.graph.regions.flatMap((r) => r.orphans));
    return (
        <>
            <ByKey each={orphans()}>
                {(orphan, key) => (
                    <Show when={props.nodes.get(key)}>
                        {(at) => <OrphanNode orphan={orphan()} at={at()} selected={props.selected === key} onSelect={props.onSelect} />}
                    </Show>
                )}
            </ByKey>
        </>
    );
}

function OrphanRows(props: { orphan: AtlasOrphan }): JSX.Element {
    return (
        <>
            <dt>State</dt>
            <dd>{containerLine(props.orphan)}</dd>
            <dt>Image</dt>
            <dd>{props.orphan.image}</dd>
            <dt>Agent</dt>
            <dd>{props.orphan.helper ?? "None: no agent of this machine uses it."}</dd>
        </>
    );
}

/** The card of the selected node when it is not an agent; nothing when `selected` is no such node. */
export function ExtraCard(props: { graph: AtlasGraph; selected: string | null; onClose: () => void }): JSX.Element {
    const orphan = () => props.graph.regions.flatMap((r) => r.orphans).find((o) => o.key === props.selected);
    const name = () => orphan()?.name;
    return (
        <Show when={name()}>
            {(title) => (
                <aside class="atlas-card" data-testid="atlas-card" aria-label={`${title()} details`}>
                    <header class="atlas-card__head">
                        <i class="fa-solid fa-box atlas-node__kind" aria-hidden="true" />
                        <strong class="atlas-card__name">{title()}</strong>
                        <CloseButton size="sm" onClick={props.onClose} />
                    </header>
                    <dl class="atlas-card__rows">
                        <Show when={orphan()}>{(o) => <OrphanRows orphan={o()} />}</Show>
                    </dl>
                </aside>
            )}
        </Show>
    );
}
