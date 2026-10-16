// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agents' requests to upgrade AgentMux, as a prompt in every window
 * (docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md §5.1).
 *
 * srv holds each request (`app:upgraderequests` event, `app.upgrade-requests`
 * RPC) that an agent made without the user's own turn asking for it. The
 * answer goes to srv the one way an approval can, through the host
 * (`approvals.decideUpgrade`). Upgrade now starts the download and the
 * restart (the launcher still asks once more before it restarts); Later
 * declines. Either way srv tells the agent.
 */

import { createEffect, createRoot, createSignal, on, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { modalsModel, type ModalCloseProps } from "@/app/store/modalmodel";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { getApi } from "@/app/store/app-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { UpgradeRequest } from "@/types/rpc/UpgradeRequest";

const [requests, setRequests] = createSignal<UpgradeRequest[]>([]);

/** The requests waiting for the user, as srv last said. */
export { requests as upgradeRequests };

function UpgradeRequestPrompt(props: { request: UpgradeRequest } & ModalCloseProps): JSX.Element {
    const r = props.request;
    const [error, setError] = createSignal<string | null>(null);
    // Answered elsewhere (another window): this prompt goes.
    createEffect(() => {
        if (!requests().some((x) => x.id === r.id)) props.close();
    });
    const decide = async (accept: boolean) => {
        try {
            await getApi().approvals.decideUpgrade!(r.id, r.version, accept);
            setRequests((list) => list.filter((x) => x.id !== r.id));
            props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <ConfirmModal
            open={true}
            title={`${r.agent} asks to upgrade AgentMux to ${r.version}`}
            attention
            confirmLabel="Upgrade now"
            cancelLabel="Later"
            onConfirm={() => decide(true)}
            onCancel={() => void decide(false)}
        >
            <p class="upgrade-request-reason">{r.reason}</p>
            <p class="upgrade-request-note">
                AgentMux downloads it, then restarts once every agent has finished its turn, after you confirm once
                more. Agents resume their conversations afterwards.
            </p>
            {error() && (
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            )}
        </ConfirmModal>
    );
}

let started = false;

/** Follow srv's upgrade requests and open a prompt for each new one. Call once. */
export function startUpgradeRequests(): void {
    // Only a host that can relay the answer can show the prompt.
    if (started || typeof getApi()?.approvals?.decideUpgrade !== "function") return;
    started = true;
    const shown = new Set<string>();
    // An event is newer than the first fetch: once one arrives, the fetch's
    // answer is stale and is dropped.
    let sawEvent = false;
    onMuxEvent(WpsEvent.AppUpgradeRequests, (data) => {
        sawEvent = true;
        setRequests(data.requests);
    });
    createRoot(() =>
        createEffect(
            on(requests, (list) => {
                const live = new Set(list.map((r) => r.id));
                for (const id of [...shown]) if (!live.has(id)) shown.delete(id);
                for (const r of list) {
                    if (shown.has(r.id)) continue;
                    shown.add(r.id);
                    modalsModel.openModal(UpgradeRequestPrompt, { request: r });
                }
            })
        )
    );
    void RpcApi.AppUpgradeRequestsCommand(TabRpcClient)
        .then((r) => {
            if (!sawEvent) setRequests(r.requests);
        })
        .catch((e) => console.log("upgrade requests: could not load them", e));
}
