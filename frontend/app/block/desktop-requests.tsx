// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent asking to drive a native app, as a card in its own pane
 * (docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md §5.4; the card, SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md
 * row 4). A request that names no pane is still asked as a window prompt.
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
import { paneRequests, type PaneRequestHandle } from "@/app/store/pane-requests";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { getApi } from "@/util/app-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DesktopGrantView } from "@/types/rpc/DesktopGrantView";
import type { DesktopRequest } from "@/types/rpc/DesktopRequest";
import { DesktopRequestCard } from "./desktop-request-card";

export type DesktopChoice = "timed" | "session" | "deny";

const [requests, setRequests] = createSignal<DesktopRequest[]>([]);
const [grants, setGrants] = createSignal<DesktopGrantView[]>([]);

/** The requests waiting for the user, as srv last said. */
export { requests as desktopRequests };
/** The apps agents may drive right now, as srv last said. */
export { grants as desktopGrants };

/** Sends the answer to srv through the host, and takes the request off the list. */
async function decide(r: DesktopRequest, choice: DesktopChoice): Promise<void> {
    await getApi().approvals.decideDesktop!(r.id, choice);
    setRequests((list) => list.filter((x) => x.id !== r.id));
}

function DesktopRequestPrompt(props: { request: DesktopRequest } & ModalCloseProps): JSX.Element {
    const r = props.request;
    const [error, setError] = createSignal<string | null>(null);
    // Answered elsewhere (another window): this prompt goes.
    createEffect(() => {
        if (!requests().some((x) => x.id === r.id)) props.close();
    });
    const answer = async (choice: DesktopChoice) => {
        try {
            await decide(r, choice);
            props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <Modal open={true} operatorOnly onClose={() => void answer("deny")} size="sm">
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
                <Button data-modal-dismiss onClick={() => void answer("deny")}>
                    Don't allow
                </Button>
                <Button onClick={() => void answer("session")}>Allow for this session</Button>
                <Button tone="attention" onClick={() => void answer("timed")}>
                    {`Allow for ${r.grant_minutes} min`}
                </Button>
            </ModalFooter>
        </Modal>
    );
}

function showCard(r: DesktopRequest): PaneRequestHandle {
    return paneRequests.push(r.block_id, {
        source: "desktop-control",
        Component: DesktopRequestCard,
        props: { request: r, decide: (choice: DesktopChoice) => decide(r, choice) },
        summary: `${r.agent}: control ${r.app}`,
        waits: false,
        // The pane was closed with the question open: No.
        onGone: () => void decide(r, "deny").catch(() => {}),
    });
}

function showPrompt(r: DesktopRequest): null {
    modalsModel.openModal(DesktopRequestPrompt, { request: r });
    return null;
}

let started = false;

/** Follow srv's desktop requests and open a prompt for each new one. Call once. */
export function startDesktopRequests(): void {
    // Only a host that can relay the answer can show the prompt.
    if (started || typeof getApi()?.approvals?.decideDesktop !== "function") return;
    started = true;
    const shown = new Map<string, PaneRequestHandle | null>();
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
                for (const [id, card] of [...shown]) {
                    if (live.has(id)) continue;
                    card?.close();
                    shown.delete(id);
                }
                for (const r of list) {
                    if (shown.has(r.id)) continue;
                    // The card sits in every window's registry and shows in the one that has the pane;
                    // srv's attention event for the pane gives the tone and the notification.
                    shown.set(r.id, r.block_id ? showCard(r) : showPrompt(r));
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
