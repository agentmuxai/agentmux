// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's Vault requests as cards in its own pane, docked above the composer
 * (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4):
 * a use of an entry (a fill, a command, a capture, a sign-in, a rotation) and a
 * request for a new entry. Each card is drawn from srv's own record of the
 * request, which names the pane that asked (`block_id`), never from anything in
 * the transcript, and goes when srv's list no longer has it. The tone, badges
 * and notification come from srv's attention event for the same pane, so a card
 * adds no wait of its own. A request with no pane is still asked in the window
 * (`vault-use-prompts`, `vault-request-prompts`).
 */

import { createEffect, createRoot, on, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { openPaneModal, paneRequests, type PaneRequestHandle, type PaneRequestProps } from "@/app/store/pane-requests";
import { VaultEntryForm } from "@/app/view/vault/vault-entry-form";
import { vaultKind } from "@/app/view/vault/vault-kinds";
import { declineRequest, vaultRequests, vaultUses } from "@/app/view/vault/vault-store";
import { createUseAnswer, promptTitle, UseQuestion } from "@/app/view/vault/vault-use-question";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import "./vault-request-card.scss";

function CardBody(props: { children: JSX.Element }): JSX.Element {
    return <div class="vault-request-card-body">{props.children}</div>;
}

function CardActions(props: { children: JSX.Element }): JSX.Element {
    return <div class="vault-request-card-actions">{props.children}</div>;
}

/** An agent's use of an entry: the same question as the window's prompt, never
 *  taking the focus from the composer (spec §4.4). */
export function VaultUseCard(props: { use: UseRequest } & PaneRequestProps): JSX.Element {
    const answer = createUseAnswer(props.use, props.close);
    return (
        <section class="vault-request-card" aria-label={promptTitle(props.use)}>
            <h3 class="vault-request-card-title">{promptTitle(props.use)}</h3>
            <UseQuestion use={props.use} answer={answer} Body={CardBody} Actions={CardActions} focus={false} />
        </section>
    );
}

/** The entry's form for a request, covering only the agent's pane. */
function RequestForm(props: { request: VaultRequest } & PaneRequestProps): JSX.Element {
    const section = vaultKind(props.request.kind)?.section ?? "api-keys";
    return <VaultEntryForm target={{ mode: "new", section, request: props.request }} onClose={() => props.close()} />;
}

/** An agent's request for a new entry: what it asks for and why, in its words.
 *  "Add it" opens the form in the pane; the value is typed there, never here. */
export function VaultRequestCard(props: { request: VaultRequest } & PaneRequestProps): JSX.Element {
    const r = props.request;
    const kind = vaultKind(r.kind);
    return (
        <section class="vault-request-card" aria-label={`${r.agent} asks you to add ${r.label}`}>
            <h3 class="vault-request-card-title">
                {r.agent} asks you to add <strong>{r.label}</strong> ({kind?.label ?? r.kind})
            </h3>
            <div class="vault-request-card-body">
                {r.why && (
                    <p>
                        {r.agent}'s reason: <q class="vault-request-card-why">{r.why}</q>
                    </p>
                )}
                <p class="warden-manager-dim">{"The agent gets the entry's name to use, never its value."}</p>
            </div>
            <div class="vault-request-card-actions">
                <Button tone="quiet" onClick={() => void declineRequest(r.id).catch(() => {})}>
                    Not now
                </Button>
                <Button tone="accent" onClick={() => openPaneModal(r.block_id, RequestForm, { request: r }, { source: "vault-request-form", waits: false })}>
                    Add it
                </Button>
            </div>
        </section>
    );
}

/** Keeps one card per item of `list()` that names a pane, for as long as srv's
 *  list has it. */
function follow<T extends { id: string; block_id: string }>(list: () => T[], push: (item: T) => PaneRequestHandle): void {
    const shown = new Map<string, PaneRequestHandle>();
    createEffect(
        on(list, (items) => {
            const live = new Set(items.map((x) => x.id));
            for (const [id, card] of shown) {
                if (live.has(id)) continue;
                card.close();
                shown.delete(id);
            }
            for (const item of items) {
                if (!item.block_id || shown.has(item.id)) continue;
                shown.set(item.id, push(item));
            }
        })
    );
}

let started = false;

/** Shows every Vault request that names a pane as a card there. Call once per window. */
export function startVaultRequestCards(): void {
    if (started) return;
    started = true;
    createRoot(() => {
        follow(vaultUses, (u) =>
            paneRequests.push(u.block_id, { source: "vault-use", Component: VaultUseCard, props: { use: u }, summary: u.entry_label, deadlineMs: u.goes_ahead ? u.deadline_ms : undefined, waits: false })
        );
        follow(vaultRequests, (r) => paneRequests.push(r.block_id, { source: "vault-request", Component: VaultRequestCard, props: { request: r }, summary: r.label, waits: false }));
    });
}

/** Tests only. */
export function __resetVaultRequestCards(): void {
    started = false;
}
