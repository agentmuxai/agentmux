// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's requests as cards, docked above the composer beside the
 * question and permission panels
 * (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4.1, §7).
 * Outside the scrolling transcript, so nothing an agent prints sits where a
 * card does, and marked operator-only, so an agent's UI query, click and key
 * presses never reach one. One card is shown in full, the first to come unless
 * the person picks another; the rest are a line each under it. The dock never
 * takes the focus: the person typing in the composer keeps typing.
 *
 * `PaneRequestModals` draws the pane's `modal` requests; it sits inside the
 * pane's `<ModalLayer scope="pane">`, so each covers only that pane.
 */

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";

import { OperatorOnlyModals } from "@/app/element/modal";
import { Button } from "@/app/element/ui";

import { paneRequests, type PaneRequest } from "@/app/store/pane-requests";
import "./pane-requests.scss";

function Drawn(props: { request: PaneRequest }): JSX.Element {
    return <Dynamic component={props.request.Component} {...props.request.props} close={() => paneRequests.close(props.request.id)} />;
}

export function PaneRequestDock(props: { blockId: string }): JSX.Element {
    const cards = createMemo(() => paneRequests.of(props.blockId).filter((r) => r.presentation === "card"));
    const [picked, setPicked] = createSignal<number | null>(null);
    const shown = createMemo(() => cards().find((r) => r.id === picked()) ?? cards()[0]);
    const rest = createMemo(() => cards().filter((r) => r !== shown()));
    return (
        <Show when={shown()}>
            <div class="pane-request-dock" data-testid="pane-request-dock" data-operator-only="" role="region" aria-label="Waiting for you">
                <Show when={shown()} keyed>
                    {(r) => <Drawn request={r} />}
                </Show>
                <Show when={rest().length > 0}>
                    <div class="pane-request-more" data-testid="pane-request-more">
                        <span class="pane-request-more-count">{rest().length} more:</span>
                        <For each={rest()}>
                            {(r) => (
                                <Button tone="quiet" class="pane-request-more-item" onClick={() => setPicked(r.id)}>
                                    {r.summary ?? r.source}
                                </Button>
                            )}
                        </For>
                    </div>
                </Show>
            </div>
        </Show>
    );
}

/** A modal renders through a portal, outside anything this host draws, so its
 *  operator-only mark comes from the context every `Modal` reads. */
export function PaneRequestModals(props: { blockId: string }): JSX.Element {
    const modals = createMemo(() => paneRequests.of(props.blockId).filter((r) => r.presentation === "modal"));
    return (
        <OperatorOnlyModals.Provider value={true}>
            <For each={modals()}>{(r) => <Drawn request={r} />}</For>
        </OperatorOnlyModals.Provider>
    );
}
