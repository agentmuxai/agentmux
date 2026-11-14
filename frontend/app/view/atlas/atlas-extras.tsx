// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What is on the Atlas map and is not an agent (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.4): the review
 * bot and CI, as two nodes in a region of their own, and a container of this machine that no agent uses, as a node
 * in the machine's region. Each opens a card, as an agent's node does; none can be selected for a fleet action.
 */

import { CloseButton } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { clockTime } from "@/app/store/muxbus-delivery";
import { createMemo, For, Show, type JSX } from "solid-js";
import { containerLine } from "./atlas-card-facts";
import type { AtlasGraph, AtlasOrphan, AtlasService } from "./atlas-graph";
import { NODE_H, NODE_W, type Placed, type RegionBox } from "./atlas-layout";

const SERVICE_STATES: Record<AtlasService["state"], string> = {
    ok: "Nothing needs attention",
    warn: "Needs attention",
    paused: "Notices are not arriving",
};

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

function ServiceNode(props: { service: AtlasService; at: Placed; selected: boolean; onSelect: Select }): JSX.Element {
    const s = () => props.service;
    return (
        <div
            {...nodeProps(s().key, props.at, props.selected, props.onSelect)}
            class={`atlas-node atlas-node--service atlas-node--service-${s().state}`}
            classList={{ "atlas-node--selected": props.selected }}
            data-testid="atlas-service"
            aria-label={`${s().name}, ${s().sub}`}
            {...tip(`${SERVICE_STATES[s().state]}: ${s().sub}`)}
        >
            <span class="atlas-node__top">
                <span class="atlas-node__dot" />
                <span class="atlas-node__name">{s().name}</span>
                <i class={`fa-solid fa-${s().icon} atlas-node__kind`} aria-hidden="true" />
            </span>
            <span class="atlas-node__sub">{s().sub}</span>
        </div>
    );
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
            aria-label={`${o().name}, a container no agent uses, ${o().state}`}
            {...tip(`A container no agent uses, ${containerLine(o())}`)}
        >
            <span class="atlas-node__top">
                <i class="fa-solid fa-box atlas-node__kind" aria-hidden="true" />
                <span class="atlas-node__name">{o().name}</span>
            </span>
            <span class="atlas-node__sub">No agent · {o().state}</span>
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
            <For each={orphans()}>
                {(orphan) => (
                    <Show when={props.nodes.get(orphan.key)}>
                        {(at) => <OrphanNode orphan={orphan} at={at()} selected={props.selected === orphan.key} onSelect={props.onSelect} />}
                    </Show>
                )}
            </For>
            <For each={props.graph.services}>
                {(service) => (
                    <Show when={props.nodes.get(service.key)}>
                        {(at) => (
                            <ServiceNode service={service} at={at()} selected={props.selected === service.key} onSelect={props.onSelect} />
                        )}
                    </Show>
                )}
            </For>
        </>
    );
}

/** The box the review bot and CI stand in, in the By machine layout. */
export function ServicesRegion(props: { box: RegionBox }): JSX.Element {
    const box = () => props.box;
    return (
        <section
            class="atlas-region atlas-region--services"
            data-testid="atlas-services"
            style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().w}px`, height: `${box().h}px` }}
        >
            <header class="atlas-region__head">
                <span class="atlas-region__title">Pull requests</span>
                <span class="atlas-region__badge">GitHub</span>
            </header>
        </section>
    );
}

function ServiceRows(props: { service: AtlasService }): JSX.Element {
    return (
        <>
            <dt>State</dt>
            <dd>{props.service.sub}</dd>
            <dt>Pull requests</dt>
            <dd>
                <Show when={props.service.lines.length > 0} fallback="None on the map now. A line stays until its pull request merges.">
                    <ul class="atlas-card__tasks" data-testid="atlas-card-prs">
                        <For each={props.service.lines}>
                            {(line) => (
                                <li class={`atlas-card__pr atlas-card__pr--${line.tone}`}>
                                    <strong>{line.pr}</strong> {line.state}
                                    <span class="atlas-card__note">
                                        {line.agent}, {clockTime(line.atMs)}
                                    </span>
                                </li>
                            )}
                        </For>
                    </ul>
                </Show>
            </dd>
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
            <dd>None: no agent of this machine uses it.</dd>
        </>
    );
}

/** The card of the selected node when it is not an agent; nothing when `selected` is no such node. */
export function ExtraCard(props: { graph: AtlasGraph; selected: string | null; onClose: () => void }): JSX.Element {
    const service = () => props.graph.services.find((s) => s.key === props.selected);
    const orphan = () => props.graph.regions.flatMap((r) => r.orphans).find((o) => o.key === props.selected);
    const name = () => service()?.name ?? orphan()?.name;
    return (
        <Show when={name()}>
            {(title) => (
                <aside class="atlas-card" data-testid="atlas-card" aria-label={`${title()} details`}>
                    <header class="atlas-card__head">
                        <i class={`fa-solid fa-${service()?.icon ?? "box"} atlas-node__kind`} aria-hidden="true" />
                        <strong class="atlas-card__name">{title()}</strong>
                        <CloseButton size="sm" onClick={props.onClose} />
                    </header>
                    <dl class="atlas-card__rows">
                        <Show when={service()}>{(s) => <ServiceRows service={s()} />}</Show>
                        <Show when={orphan()}>{(o) => <OrphanRows orphan={o()} />}</Show>
                    </dl>
                </aside>
            )}
        </Show>
    );
}
