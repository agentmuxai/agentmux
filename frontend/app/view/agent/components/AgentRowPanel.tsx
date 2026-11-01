// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The panel under a My agents tile: the New Agent panel's own look (a full-width row under the tile's
 * row, an accent frame, a caret pointing up at the tile) holding whatever the tile's agent is being asked
 * about: its actions, a rename, a fork name, "already open", Seat, Bundles, Environment, delete. The
 * caller picks the body; this is the frame, the header, the caret and the scroll that keeps the
 * panel's bottom in view when it opens or changes.
 */

import { IconButton } from "@/app/element/ui";
import "@/app/element/ui/card.scss";
import { revealDetail } from "@/app/element/ui/detail-reveal";
import { createEffect, createSignal, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import "./AgentCreatePanel.scss";
import "./AgentRowPanel.scss";

export interface AgentRowPanelProps {
    /** The agent's name. */
    title: string;
    /** What the panel is showing now ("Rename", "Seat"), after the name. */
    subtitle?: string;
    /** The tile the panel hangs from: the caret points at its middle, and it is kept in view above the panel. */
    tile: () => HTMLElement | undefined;
    /** Changes when the panel shows something else, so the pane scrolls to its new bottom. */
    viewKey: string;
    onClose: () => void;
    /** Shown as a Back button when set (a sub-view returning to the actions). */
    onBack?: () => void;
    children: JSX.Element;
}

/** How long after a view change growing content (a form loading its data) keeps the panel in view. */
const SETTLE_MS = 2000;

export const AgentRowPanel = (props: AgentRowPanelProps): JSX.Element => {
    let el: HTMLLIElement | undefined;
    // The caret's place along the panel, as a share of its width: a percentage, so chrome zoom needs no maths.
    const [caret, setCaret] = createSignal("50%");
    const place = () => {
        const tile = props.tile();
        if (!el || !tile) return;
        const t = tile.getBoundingClientRect();
        const p = el.getBoundingClientRect();
        if (p.width <= 0) return;
        const share = (t.left + t.width / 2 - p.left) / p.width;
        setCaret(`${Math.min(Math.max(share, 0.02), 0.98) * 100}%`);
    };
    const reveal = () => {
        const tile = props.tile();
        if (el && tile) revealDetail(el, tile);
    };

    let settleUntil = 0;
    onMount(() => {
        const frame = requestAnimationFrame(() => {
            place();
            reveal();
        });
        settleUntil = performance.now() + SETTLE_MS;
        onCleanup(() => cancelAnimationFrame(frame));
        if (typeof ResizeObserver === "undefined" || !el) return;
        // Its own growth keeps its bottom in view for a moment after it opened or changed; the tile's row
        // moving (a resize, a column change) only moves the caret.
        const ro = new ResizeObserver(() => {
            place();
            if (performance.now() < settleUntil) reveal();
        });
        ro.observe(el);
        const list = el.parentElement;
        if (list) ro.observe(list);
        onCleanup(() => ro.disconnect());
    });
    createEffect(
        on(
            () => props.viewKey,
            () => {
                settleUntil = performance.now() + SETTLE_MS;
                requestAnimationFrame(() => {
                    place();
                    reveal();
                });
            },
            { defer: true }
        )
    );

    return (
        <li
            ref={el}
            class="ui-card-detail agent-row-panel"
            style={{ "--ui-card-caret-left": caret() }}
            data-testid="agent-row-panel"
            data-view={props.viewKey}
            // The panel opens with a short move (card.scss); its first measure is mid-move, so measure again at the end.
            onAnimationEnd={reveal}
        >
            <section class="agent-create-panel" aria-label={props.title}>
                <header class="agent-create-panel-header">
                    <h3 class="agent-create-panel-title">
                        {props.title}
                        <Show when={props.subtitle}>
                            <span class="agent-row-panel-subtitle"> · {props.subtitle}</span>
                        </Show>
                    </h3>
                    <span class="agent-row-panel-header-actions">
                        <Show when={props.onBack}>
                            {(back) => (
                                <IconButton
                                    icon="arrow-left"
                                    label="Back"
                                    density="compact"
                                    onClick={() => back()()}
                                    data-testid="agent-row-panel-back"
                                />
                            )}
                        </Show>
                        <IconButton
                            icon="xmark"
                            label="Close"
                            density="compact"
                            onClick={() => props.onClose()}
                            data-testid="agent-row-panel-close"
                        />
                    </span>
                </header>
                {props.children}
            </section>
        </li>
    );
};

AgentRowPanel.displayName = "AgentRowPanel";
