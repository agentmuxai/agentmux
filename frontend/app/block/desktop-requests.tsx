// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent asking to drive a native app, as a prompt in every window
 * (docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md §5.4).
 *
 * srv holds each request (`app:desktop` event, `app.desktop` RPC) while the
 * agent's `Desktop` call waits. The answer goes to srv the one way an approval
 * can, through the host (`approvals.decideDesktop`), so no agent can approve
 * its own request. Closing the prompt without choosing is a No.
 */

import { createEffect, createRoot, createSignal, on, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button } from "@/app/element/ui";
import { modalsModel, type ModalCloseProps } from "@/app/store/modalmodel";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { getApi } from "@/app/store/app-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DesktopGrantView } from "@/types/rpc/DesktopGrantView";
import type { DesktopRequest } from "@/types/rpc/DesktopRequest";

export type DesktopChoice = "timed" | "session" | "deny";

const [requests, setRequests] = createSignal<DesktopRequest[]>([]);
const [grants, setGrants] = createSignal<DesktopGrantView[]>([]);

/** The requests waiting for the user, as srv last said. */
export { requests as desktopRequests };
/** The apps agents may drive right now, as srv last said. */
export { grants as desktopGrants };

function DesktopRequestPrompt(props: { request: DesktopRequest } & ModalCloseProps): JSX.Element {
    const r = props.request;
    const [error, setError] = createSignal<string | null>(null);
    // Answered elsewhere (another window): this prompt goes.
    createEffect(() => {
        if (!requests().some((x) => x.id === r.id)) props.close();
    });
    const decide = async (choice: DesktopChoice) => {
        try {
            await getApi().approvals.decideDesktop!(r.id, choice);
            setRequests((list) => list.filter((x) => x.id !== r.id));
            props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <Modal open={true} operatorOnly onClose={() => void decide("deny")} size="sm">
            <ModalHeader title={`${r.agent} wants to control ${r.app}`} />
            <ModalBody>
                <p class="desktop-request-window">{r.title ? `Window: ${r.title}` : null}</p>
                <p class="desktop-request-note">
                    It will see this window and be able to click and type in it. You can stop it at any time with Stop in
                    its pane. Everything it does is logged.
                </p>
                {error() && (
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                )}
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={() => void decide("deny")}>
                    Don't allow
                </Button>
                <Button onClick={() => void decide("session")}>Allow for this session</Button>
                <Button tone="attention" onClick={() => void decide("timed")}>
                    {`Allow for ${r.grant_minutes} min`}
                </Button>
            </ModalFooter>
        </Modal>
    );
}

let started = false;

/** Follow srv's desktop requests and open a prompt for each new one. Call once. */
export function startDesktopRequests(): void {
    // Only a host that can relay the answer can show the prompt.
    if (started || typeof getApi()?.approvals?.decideDesktop !== "function") return;
    started = true;
    const shown = new Set<string>();
    // An event is newer than the first fetch: once one arrives, the fetch's
    // answer is stale and is dropped.
    let sawEvent = false;
    onMuxEvent(WpsEvent.AppDesktop, (data) => {
        sawEvent = true;
        setRequests(data.requests);
        setGrants(data.grants);
    });
    createRoot(() =>
        createEffect(
            on(requests, (list) => {
                const live = new Set(list.map((r) => r.id));
                for (const id of [...shown]) if (!live.has(id)) shown.delete(id);
                for (const r of list) {
                    if (shown.has(r.id)) continue;
                    shown.add(r.id);
                    modalsModel.openModal(DesktopRequestPrompt, { request: r });
                }
            })
        )
    );
    void RpcApi.AppDesktopCommand(TabRpcClient)
        .then((r) => {
            if (!sawEvent) {
                setRequests(r.requests);
                setGrants(r.grants);
            }
        })
        .catch((e) => console.log("desktop requests: could not load them", e));
}
