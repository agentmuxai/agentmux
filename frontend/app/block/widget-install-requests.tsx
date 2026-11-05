// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agents' requests to install a widget, as a card in the asking agent's pane, or a prompt for the user
 * when the request names no pane (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §10;
 * docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 5).
 *
 * srv holds each request (`widgetrequests` event, `widgets.requests` RPC)
 * while the agent's `WidgetInstall` waits. The prompt shows what Settings →
 * Widgets would, and the answer goes to srv the one way an approval can,
 * through the host (`approvals.decideWidget`); the agent hears it from srv.
 */

import { createEffect, createRoot, createSignal, on, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { modalsModel, type ModalCloseProps } from "@/app/store/modalmodel";
import { paneRequests, type PaneRequestHandle } from "@/app/store/pane-requests";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import type { WidgetInstallRequest } from "@/app/store/rpc-api/widgets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi } from "@/util/app-api";
import { installLabel, WidgetApprovalDetails } from "@/app/view/settings/sections/widget-approval-details";
import { WidgetInstallCard } from "./widget-install-card";

const [requests, setRequests] = createSignal<WidgetInstallRequest[]>([]);

// The signer is part of a version: a re-signed package is a new prompt.
const keyOf = (r: Pick<WidgetInstallRequest, "id" | "hash" | "signature">) => `${r.id}@${r.hash}@${r.signature?.fingerprint ?? ""}`;

/** Sends the answer to srv through the host, the one way an approval can. */
const decide = (r: WidgetInstallRequest, approve: boolean): Promise<void> =>
    getApi().approvals.decideWidget(r.id, r.hash, approve, r.signature?.fingerprint ?? "");

function InstallRequestPrompt(props: { request: WidgetInstallRequest } & ModalCloseProps): JSX.Element {
    const r = props.request;
    const [error, setError] = createSignal<string | null>(null);
    // Answered elsewhere (Settings, another window): this prompt goes.
    createEffect(() => {
        if (!requests().some((x) => keyOf(x) === keyOf(r))) props.close();
    });
    const answer = async (approve: boolean) => {
        try {
            await decide(r, approve);
            props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <ConfirmModal
            open={true}
            title={`${r.agent} wants to install ${r.name}`}
            attention
            operatorOnly
            // Not signed by the publisher's usual key: the risky choice isn't
            // the bright one, and focus starts on "Don't install".
            destructive={r.signature?.state === "key_changed"}
            confirmLabel={installLabel(r)}
            cancelLabel="Don't install"
            onConfirm={() => answer(true)}
            onCancel={() => void answer(false)}
        >
            <WidgetApprovalDetails pkg={r} />
            {error() && (
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            )}
        </ConfirmModal>
    );
}

// The card sits in every window's registry and shows in the one that has the pane; srv's attention event for
// the pane gives the tone and the notification.
function showCard(r: WidgetInstallRequest): PaneRequestHandle {
    return paneRequests.push(r.block_id, {
        source: "widget-install",
        Component: WidgetInstallCard,
        props: { request: r, decide: (approve: boolean) => decide(r, approve) },
        summary: `${r.agent}: install ${r.name}`,
        waits: false,
        // The pane was closed with the question open: No.
        onGone: () => void decide(r, false).catch(() => {}),
    });
}

function showPrompt(r: WidgetInstallRequest): null {
    modalsModel.openModal(InstallRequestPrompt, { request: r });
    return null;
}

let started = false;

/** Follow srv's requests and open a prompt for each new one. Call once. */
export function startWidgetInstallRequests(): void {
    if (started || typeof getApi()?.approvals?.decideWidget !== "function") return;
    started = true;
    const shown = new Map<string, PaneRequestHandle | null>();
    // An event is newer than the first fetch: once one arrives, the fetch's
    // answer is stale and is dropped.
    let sawEvent = false;
    onMuxEvent(WpsEvent.WidgetRequests, (data) => {
        sawEvent = true;
        setRequests(data.requests);
    });
    createRoot(() =>
        createEffect(
            on(requests, (list) => {
                const live = new Set(list.map(keyOf));
                for (const [k, card] of [...shown]) {
                    if (live.has(k)) continue;
                    card?.close();
                    shown.delete(k);
                }
                for (const r of list) {
                    if (shown.has(keyOf(r))) continue;
                    shown.set(keyOf(r), r.block_id ? showCard(r) : showPrompt(r));
                }
            })
        )
    );
    void RpcApi.WidgetsRequestsCommand(TabRpcClient)
        .then((r) => {
            if (!sawEvent) setRequests(r.requests);
        })
        .catch((e) => console.log("widget install requests: could not load them", e));
}
