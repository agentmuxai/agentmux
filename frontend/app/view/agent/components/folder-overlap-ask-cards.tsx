// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's ask for the go-ahead with a host agent that shares a folder with a container agent, as a card in the
 * asking agent's pane, above its composer (`backend/folder_overlap_asks.rs`; the same card dock as an agent's Vault
 * request). srv keeps each ask (`folderoverlapasks` event, `folderoverlapasks` RPC) and the person answers with one
 * click: Go ahead is `folderoverlapaccept` and Don't open is `folderoverlapdecline`, which only this window can make.
 * The card is drawn from srv's own record of the ask, never from words the agent wrote, and goes when srv's list no
 * longer has the pair.
 */

import { createEffect, createRoot, createSignal, on, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { paneRequests, type PaneRequestHandle, type PaneRequestProps } from "@/app/store/pane-requests";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { declineSharedFolder, goAheadWithSharedFolder } from "@/app/view/agent/shared-folder-ask";
import type { FolderAsk } from "@/types/rpc/FolderAsk";
import "./vault-request-card/vault-request-card.scss";

export interface FolderOverlapAskCardProps extends PaneRequestProps {
    ask: FolderAsk;
    goAhead: () => Promise<void>;
    decline: () => Promise<void>;
}

/** The ask: who asks, which two agents share a folder, what that means, and the two answers. */
export function FolderOverlapAskCard(props: FolderOverlapAskCardProps): JSX.Element {
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const answer = async (send: () => Promise<void>) => {
        setBusy(true);
        setError(null);
        try {
            await send();
            props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
            setBusy(false);
        }
    };
    const title = () => t("agentPicker.sharedFolder.askTitle", { agent: props.ask.agent, host: props.ask.host_name });
    return (
        <section class="vault-request-card" aria-label={title()} data-testid="folder-overlap-ask-card">
            <h3 class="vault-request-card-title">{title()}</h3>
            <div class="vault-request-card-body">
                <p data-testid="folder-overlap-ask-text">
                    {t("agentPicker.sharedFolder.body", { containers: props.ask.container_name, host: props.ask.host_name })}
                </p>
                <p class="warden-manager-dim">{t("agentPicker.sharedFolder.hint")}</p>
                <Show when={error()}>
                    <p role="alert">{error()}</p>
                </Show>
            </div>
            <div class="vault-request-card-actions">
                <Button tone="quiet" disabled={busy()} onClick={() => answer(props.decline)} data-testid="folder-overlap-ask-decline">
                    {t("agentPicker.sharedFolder.cancel")}
                </Button>
                <Button tone="accent" busy={busy()} onClick={() => answer(props.goAhead)} data-testid="folder-overlap-ask-go-ahead">
                    {t("agentPicker.sharedFolder.goAhead")}
                </Button>
            </div>
        </section>
    );
}

const [asks, setAsks] = createSignal<FolderAsk[]>([]);

const keyOf = (ask: Pick<FolderAsk, "host_id" | "container_id">) => `${ask.host_id}:${ask.container_id}`;

function showCard(ask: FolderAsk): PaneRequestHandle {
    return paneRequests.push(ask.block_id, {
        source: "folder-overlap-ask",
        operatorOnly: true,
        Component: FolderOverlapAskCard,
        props: { ask, goAhead: () => goAheadWithSharedFolder([ask]), decline: () => declineSharedFolder(ask) },
        what: "asks you to open an agent",
        summary: `${ask.agent}: open ${ask.host_name}`,
    });
}

/**
 * One card per ask of srv's list, in the asking agent's pane, for as long as the list has it; a pair the newest asker
 * moved to another pane has its card there. `shown` is what is on screen, by pair.
 */
function follow(list: () => FolderAsk[]): void {
    const shown = new Map<string, { blockId: string; card: PaneRequestHandle }>();
    createEffect(
        on(list, (items) => {
            const live = new Set(items.map(keyOf));
            for (const [key, card] of shown) {
                if (live.has(key)) continue;
                card.card.close();
                shown.delete(key);
            }
            for (const ask of items) {
                const was = shown.get(keyOf(ask));
                if (was?.blockId === ask.block_id) continue;
                was?.card.close();
                shown.set(keyOf(ask), { blockId: ask.block_id, card: showCard(ask) });
            }
        })
    );
}

let started = false;

/** Shows every agent's ask for the go-ahead as a card in its pane. Call once per window. */
export function startFolderOverlapAsks(): void {
    if (started) return;
    started = true;
    // An event is newer than the first fetch: once one arrives, the fetch's answer is stale and is dropped.
    let sawEvent = false;
    onMuxEvent(WpsEvent.FolderOverlapAsks, (data) => {
        sawEvent = true;
        setAsks(data.asks);
    });
    createRoot(() => follow(asks));
    void RpcApi.FolderOverlapAsksCommand(TabRpcClient)
        .then((found) => {
            if (!sawEvent) setAsks(found.asks);
        })
        .catch((e) => console.log("folder overlap asks: could not load them", e));
}
