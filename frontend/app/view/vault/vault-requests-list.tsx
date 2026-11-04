// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault pane's Requests list: every request waiting on the person, from
// every agent, with the same questions, answers and countdowns as the card in
// the asking agent's pane. The one place to answer a request whose agent has no
// pane on screen (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §6, §7).

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import { vaultKind } from "./vault-kinds";
import { declineRequest, denyAllFrom, pendingFrom, vaultRequests, vaultUses } from "./vault-store";
import { CLIP, Clipped, createUseAnswer, promptTitle, UseQuestion } from "./vault-use-question";

function ItemBody(props: { children: JSX.Element }): JSX.Element {
    return <div class="vault-use-item-body">{props.children}</div>;
}

function ItemActions(props: { children: JSX.Element }): JSX.Element {
    return <div class="vault-use-item-actions">{props.children}</div>;
}

/** An agent's use of an entry, asked here as in its pane. `close` runs once it
 *  is answered; the item itself goes when srv's list drops it. */
export function VaultUseItem(props: { use: UseRequest; close?: () => void }): JSX.Element {
    const answer = createUseAnswer(props.use, () => props.close?.());
    const title = promptTitle(props.use);
    const shown = promptTitle(props.use, true);
    return (
        <section class="vault-use-item" role="group" aria-label={title} data-use={props.use.id}>
            <h4 class="vault-use-item-title" {...tip(shown === title ? undefined : title)}>
                {shown}
            </h4>
            <UseQuestion use={props.use} answer={answer} Body={ItemBody} Actions={ItemActions} focus={false} />
        </section>
    );
}

/** An agent's ask for a new entry: Fill in opens the form, Dismiss declines it. */
function VaultRequestItem(props: { request: VaultRequest; onFill: (r: VaultRequest) => void; onError: (e: string | null) => void }): JSX.Element {
    const r = props.request;
    const dismiss = async () => {
        try {
            await declineRequest(r.id);
            props.onError(null);
        } catch (e) {
            props.onError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <div class="vault-request" data-request={r.id}>
            <div class="vault-request-text">
                <strong>
                    <Clipped text={r.agent} max={CLIP.name} />
                </strong>{" "}
                asks for a {vaultKind(r.kind)?.label.toLowerCase() ?? r.kind}:{" "}
                <strong>
                    <Clipped text={r.label} max={CLIP.label} />
                </strong>
                <Show when={r.why}>
                    <div class="warden-manager-dim">
                        <Clipped text={r.why} max={CLIP.reason} />
                    </div>
                </Show>
            </div>
            <Button tone="attention" density="compact" onClick={() => props.onFill(r)}>
                Fill in
            </Button>
            <Button tone="quiet" density="compact" onClick={() => void dismiss()}>
                Dismiss
            </Button>
        </div>
    );
}

/** Every pending request, with "Deny all from <agent>" for an agent waiting on more than one. */
export function VaultRequestsList(props: { onFill: (r: VaultRequest) => void }): JSX.Element {
    const [error, setError] = createSignal<string | null>(null);
    const several = createMemo(() => [...new Set([...vaultUses().map((u) => u.agent), ...vaultRequests().map((r) => r.agent)])].filter((a) => pendingFrom(a) > 1));
    return (
        <Show when={vaultUses().length + vaultRequests().length > 0}>
            <div class="vault-requests" role="region" aria-label="Agents' requests">
                <For each={vaultUses()}>{(u) => <VaultUseItem use={u} />}</For>
                <For each={vaultRequests()}>{(r) => <VaultRequestItem request={r} onFill={props.onFill} onError={setError} />}</For>
                <Show when={several().length > 0}>
                    <div class="vault-requests-deny-all">
                        <For each={several()}>
                            {(agent) => (
                                <Button tone="danger" density="compact" onClick={() => void denyAllFrom(agent)}>
                                    Deny all from <Clipped text={agent} max={CLIP.name} />
                                </Button>
                            )}
                        </For>
                    </div>
                </Show>
                <Show when={error()}>
                    <div class="warden-manager-error">⚠ {error()}</div>
                </Show>
            </div>
        </Show>
    );
}
