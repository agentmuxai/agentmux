// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onCleanup, onMount } from "solid-js";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import type { ModalLayerApi } from "@/element/modal-layer";
import type { BrowserViewModel } from "./browser-model";

type AuthChallenge = {
    request_id: string;
    origin: string;
    host: string;
    port: number;
    realm: string;
    is_proxy: boolean;
};

/**
 * Subscribes to CEF's HTTP Basic/Digest auth challenges and wires the
 * browser-auth modal (queue + open/submit/cancel). Lives in the view (not
 * the model) because it needs `useModalLayer()` context, which is a SolidJS
 * hook. Phase α of SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md.
 */
export function useBrowserAuth(params: {
    model: BrowserViewModel;
    modalLayer: ModalLayerApi;
    diag: (msg: string) => void;
}): void {
    const { model, modalLayer, diag } = params;

    let authUnsub: (() => void) | null = null;
    let authDisposed = false;
    // Per-pane set of in-flight auth requests. ESC/backdrop dismiss tears
    // down the modal via `safeClose()` without firing the browser-auth
    // onCancel, so any remaining ids in this set need explicit cancel on
    // pane unmount AND on resolution. Tracks ids so we cancel exactly the
    // ones that didn't resolve through the submit/cancel buttons.
    const pendingAuthIds = new Set<string>();
    // FIFO queue for concurrent auth challenges. Two protected subresources
    // on the same page (or two panes in the same tab) can challenge before
    // the user resolves the first prompt; unconditional modalLayer.open
    // would replace the visible modal, and the unmounted panel's onCleanup
    // would cancel the earlier challenge — so authenticating the survivor
    // still fails the earlier requests. Queue new arrivals; open the next
    // after the active one resolves.
    const authQueue: AuthChallenge[] = [];
    let authActive = false;

    const openAuthPrompt = (c: AuthChallenge) => {
        authActive = true;
        modalLayer.open({
            kind: "browser-auth",
            blockId: model.blockId,
            requestId: c.request_id,
            origin: c.origin || `${c.host}:${c.port}`,
            realm: c.realm,
            isProxy: c.is_proxy,
            onSubmit: (username, password, save) => {
                pendingAuthIds.delete(c.request_id);
                diag(`auth-submit request_id=${c.request_id} save=${save}`);
                void invokeCommand("browser_pane_auth_submit", {
                    request_id: c.request_id,
                    username,
                    password,
                }).catch((e) => diag(`auth-submit-failed err=${String(e)}`));
                // Opt-in save — a wholly separate IPC call (not a flag on
                // browser_pane_auth_submit) so that command's contract
                // stays byte-for-byte unchanged. A save failure never
                // affects the page load: auth already succeeded via the
                // call above regardless of what happens here.
                if (save) {
                    void invokeCommand("browser_pane_auth_save", {
                        block_id: model.blockId,
                        origin: c.origin,
                        realm: c.realm,
                        is_proxy: c.is_proxy,
                        username,
                        password,
                    }).catch((e) => diag(`auth-save-failed err=${String(e)}`));
                }
                authActive = false;
                drainAuthQueue();
            },
            onCancel: () => {
                pendingAuthIds.delete(c.request_id);
                diag(`auth-cancel request_id=${c.request_id}`);
                void invokeCommand("browser_pane_auth_cancel", {
                    request_id: c.request_id,
                }).catch(() => {});
                authActive = false;
                drainAuthQueue();
            },
        });
    };
    const drainAuthQueue = () => {
        if (authActive || authQueue.length === 0) return;
        const next = authQueue.shift()!;
        // Defer to a microtask so the prior modal's onCleanup has run
        // before we mount the next one — replacing synchronously would
        // re-trigger the cleanup-cancel path the queue exists to prevent.
        queueMicrotask(() => openAuthPrompt(next));
    };

    onMount(() => {
        // Subscribe to CEF's HTTP Basic/Digest auth challenges.
        void listenEvent<{
            block_id: string;
            request_id: string;
            origin: string;
            host: string;
            port: number;
            realm: string;
            is_proxy: boolean;
        }>("browser-pane-auth-required", (payload) => {
            if (payload.block_id !== model.blockId) return;
            // Diagnostic stays high-signal: request_id + origin + realm are
            // enough to trace the prompt without logging credentials.
            diag(`auth-required request_id=${payload.request_id} origin=${JSON.stringify(payload.origin)} realm=${JSON.stringify(payload.realm)}`);
            pendingAuthIds.add(payload.request_id);
            const challenge: AuthChallenge = {
                request_id: payload.request_id,
                origin: payload.origin,
                host: payload.host,
                port: payload.port,
                realm: payload.realm,
                is_proxy: payload.is_proxy,
            };
            if (authActive) {
                diag(`auth-queue request_id=${payload.request_id} depth=${authQueue.length + 1}`);
                authQueue.push(challenge);
            } else {
                openAuthPrompt(challenge);
            }
        }).then((unsub) => {
            // listenEvent's promise can resolve AFTER onCleanup has already
            // run (pane closed before subscription completed). Without this
            // check, the unsub closure is captured post-cleanup and never
            // invoked, leaking the listener until renderer teardown.
            if (authDisposed) unsub();
            else authUnsub = unsub;
        });
    });

    onCleanup(() => {
        authDisposed = true;
        if (authUnsub) {
            authUnsub();
            authUnsub = null;
        }
        // Cancel any auth prompts still parked on the host (active +
        // queued). The backend also fires `cancel_for_block` from
        // `browser_pane_close` as a safety net, but firing them here
        // ensures each cancel logs against the correct request_id.
        for (const requestId of pendingAuthIds) {
            invokeCommand("browser_pane_auth_cancel", { request_id: requestId })
                .catch(() => {});
        }
        pendingAuthIds.clear();
        authQueue.length = 0;
        authActive = false;
    });
}
