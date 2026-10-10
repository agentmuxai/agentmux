// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agents' requests to install a widget, as a prompt for the user
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §10).
 *
 * srv holds each request (`widgetrequests` event, `widgets.requests` RPC)
 * while the agent's `WidgetInstall` waits. The prompt shows what Settings →
 * Widgets would, and the answer goes to srv the one way an approval can,
 * through the host (`approvals.decideWidget`); the agent hears it from srv.
 */

import { createEffect, createRoot, createSignal, on, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { modalsModel, type ModalCloseProps } from "@/app/store/modalmodel";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import type { WidgetInstallRequest } from "@/app/store/rpc-api/widgets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi } from "@/app/store/app-api";
import { WidgetApprovalDetails } from "@/app/view/settings/sections/widget-approval-details";

const [requests, setRequests] = createSignal<WidgetInstallRequest[]>([]);

const keyOf = (r: Pick<WidgetInstallRequest, "id" | "hash">) => `${r.id}@${r.hash}`;

function InstallRequestPrompt(props: { request: WidgetInstallRequest } & ModalCloseProps): JSX.Element {
    const r = props.request;
    const [error, setError] = createSignal<string | null>(null);
    // Answered elsewhere (Settings, another window): this prompt goes.
    createEffect(() => {
        if (!requests().some((x) => keyOf(x) === keyOf(r))) props.close();
    });
    const decide = async (approve: boolean) => {
        try {
            await getApi().approvals.decideWidget(r.id, r.hash, approve);
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
            confirmLabel="Install"
            cancelLabel="Don't install"
            onConfirm={() => decide(true)}
            onCancel={() => void decide(false)}
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

let started = false;

/** Follow srv's requests and open a prompt for each new one. Call once. */
export function startWidgetInstallRequests(): void {
    if (started || typeof getApi()?.approvals?.decideWidget !== "function") return;
    started = true;
    const shown = new Set<string>();
    // An event is newer than the first fetch: once one arrives, the fetch's
    // answer is stale and is dropped.
    let sawEvent = false;
    muxEventSubscribe({
        eventType: WpsEvent.WidgetRequests,
        handler: (event: { data?: { requests?: WidgetInstallRequest[] } }) => {
            sawEvent = true;
            setRequests(event?.data?.requests ?? []);
        },
    });
    createRoot(() =>
        createEffect(
            on(requests, (list) => {
                const live = new Set(list.map(keyOf));
                for (const k of [...shown]) if (!live.has(k)) shown.delete(k);
                for (const r of list) {
                    if (shown.has(keyOf(r))) continue;
                    shown.add(keyOf(r));
                    modalsModel.openModal(InstallRequestPrompt, { request: r });
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
