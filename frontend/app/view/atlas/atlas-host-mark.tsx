// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How a node shows its host (atlas-hosts.ts): the host's name on the node, and a ring outside its border in the
 * host's colour (atlas.scss `.atlas-node[data-host-colour]`); and the legend that sets each host's name beside its ring.
 */

import { For, Show, type JSX } from "solid-js";
import { t } from "@/app/i18n/i18n";
import type { AtlasRegion } from "./atlas-graph";
import { distinctHosts, type HostStyle } from "./atlas-hosts";
import type { Gaps } from "./atlas-layout";

/** What a node's host adds to its element: the ring's colour and line, when rings are drawn. */
export function hostAttrs(style: HostStyle | undefined, ring: boolean): { "data-host-colour"?: number; "data-host-line"?: string } {
    return style && ring ? { "data-host-colour": style.colour + 1, "data-host-line": style.line } : {};
}

/** ", on alpha": the host in words, for a node's accessible name and tooltip. */
export function onHost(style: HostStyle | undefined): string {
    return style ? `, on ${style.name}` : "";
}

/** The host's name on a node, cut with an ellipsis before the node's own name is. */
export function HostLabel(props: { style: HostStyle | undefined }): JSX.Element {
    return (
        <Show when={props.style}>
            {(s) => (
                <span class="atlas-node__host" data-testid="atlas-node-host">
                    {s().name}
                </span>
            )}
        </Show>
    );
}

/**
 * The ring's width and the gap inside it, closing with the gaps between nodes as the map is zoomed in, so two
 * neighbours' rings never meet: the gap is a seventh of the gap between two nodes (2 px at full), and the ring is
 * 2 px wide while there is room, and no less than 1 px.
 */
export function ringVars(gaps: Gaps): Record<string, string> {
    const gap = Math.max(1, Math.round((gaps.x / 7) * 10) / 10);
    const width = gaps.x >= 10 ? 2 : 1;
    return { "--atlas-ring-gap": `${gap}px`, "--atlas-ring-w": `${width}px` };
}

/** What a machine says about itself, after its host's name: how it was found, its platform, and what is wrong with it. */
export function machineDetail(r: Pick<AtlasRegion, "badge" | "platform" | "stale" | "note" | "cloud" | "title">): string {
    const channel = r.title.includes("·") ? r.title.split("·").slice(1).join("·").trim() : "";
    return [channel, r.badge, r.platform, r.stale ? "stale" : "", r.note].filter(Boolean).join(" · ");
}

/** Each host's ring beside its name and what its machines say about themselves. Not drawn for one host. */
export function AtlasLegend(props: { styles: ReadonlyMap<string, HostStyle>; regions: readonly AtlasRegion[] }): JSX.Element {
    const details = (host: HostStyle) =>
        props.regions
            .filter((r) => props.styles.get(r.key)?.id === host.id)
            .map(machineDetail)
            .filter(Boolean)
            .join("; ");
    return (
        <ul class="atlas-legend" data-testid="atlas-legend" aria-label={t("atlas.legend.hosts")}>
            <For each={distinctHosts(props.styles)}>
                {(host) => (
                    <li class="atlas-legend__host">
                        <span class="atlas-legend__ring" data-host-colour={host.colour + 1} data-host-line={host.line} aria-hidden="true" />
                        <span class="atlas-legend__name">{host.name}</span>
                        <Show when={details(host)}>
                            {(text) => <span class="atlas-legend__detail">{text()}</span>}
                        </Show>
                    </li>
                )}
            </For>
        </ul>
    );
}
