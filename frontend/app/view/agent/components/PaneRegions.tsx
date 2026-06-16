// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneRegions — the declarative region container for the agent pane.
 *
 * Replaces the hand-ordered ~16-surface JSX stack in `agent-view.tsx` with a
 * `region → content` map: surfaces *register into a named region* instead of
 * being positioned by JSX accident. Each region owns its layout contract
 * (flex / z-order / max-height) in `PaneRegions.scss`; surfaces just declare
 * which region they belong to.
 *
 * This is the §5.1 region model of
 * `docs/specs/SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md`, and the shared
 * dependency the session-digest accessory and the forthcoming fork bar both
 * register into. Presentational only — it owns no state.
 */

import { For, Show, type JSX } from "solid-js";
import "./PaneRegions.scss";

/** Named regions, top → bottom. `overlay` is z-stacked over the column (the
 *  pane's *real* layers — focused panel, pane-scoped modals); everything else
 *  is in the flex column. `stream` is the single flex-grow region. */
export type PaneRegionName =
    | "top-fixed" // transient banners (progress, search, digest)
    | "dock" // pinned long-running processes (ActivityDock)
    | "stream" // the conversation (flex: 1, scrolls)
    | "alert" // working-row · decision · question · disconnected
    | "queue" // pending messages
    | "status" // composer strip
    | "input" // details · slash · footer textarea
    | "forks" // the fork bar (conversations in this pane)
    | "overlay"; // focused panel · pane-scoped modals (z-stacked, clipped)

/** Canonical render order. The flex column flows top→bottom; `overlay` renders
 *  last so it stacks above the column. */
export const PANE_REGION_ORDER: readonly PaneRegionName[] = [
    "top-fixed",
    "dock",
    "stream",
    "alert",
    "queue",
    "status",
    "input",
    "forks",
    "overlay",
] as const;

export interface PaneRegionsProps {
    /** Content per region. A region absent from the map (or with empty content)
     *  renders no wrapper — zero layout cost for the common case. */
    regions: Partial<Record<PaneRegionName, JSX.Element | JSX.Element[]>>;
}

/** True when a region has anything renderable (a non-empty array / a node). */
function hasContent(c: JSX.Element | JSX.Element[] | undefined): boolean {
    if (c == null || c === false) return false;
    if (Array.isArray(c)) return c.some((x) => x != null && x !== false);
    return true;
}

export const PaneRegions = (props: PaneRegionsProps): JSX.Element => {
    return (
        <For each={PANE_REGION_ORDER}>
            {(name) => {
                const content = (): JSX.Element | JSX.Element[] | undefined => props.regions[name];
                return (
                    <Show when={hasContent(content())}>
                        <div class={`pane-region pane-region--${name}`} data-region={name}>
                            {content()}
                        </div>
                    </Show>
                );
            }}
        </For>
    );
};

PaneRegions.displayName = "PaneRegions";
