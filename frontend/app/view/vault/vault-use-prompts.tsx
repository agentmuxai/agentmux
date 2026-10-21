// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent's use of a Vault entry, as a prompt for the person
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6.4): which
// entry, which site, which field, and why they are being asked. The answer goes to
// srv the one way it can, through the host. Closing the prompt denies.

import { createEffect, createRoot, createSignal, on, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button } from "@/app/element/ui";
import { modalsModel, type ModalCloseProps } from "@/app/store/modalmodel";
import type { UseRequest } from "@/types/rpc/UseRequest";
import { answerUse, followVault, refreshVaultRequests, vaultAvailable, vaultUses, type UseDecision } from "./vault-store";

/** The host name of an origin, for the "Always on" button. */
export function siteOf(origin: string): string {
    try {
        return new URL(origin).host || origin;
    } catch {
        return origin;
    }
}

export function UsePrompt(props: { use: UseRequest } & ModalCloseProps): JSX.Element {
    const u = props.use;
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    // Answered elsewhere (another window), or the agent gave up: this prompt goes.
    createEffect(() => {
        if (!vaultUses().some((x) => x.id === u.id)) props.close();
    });
    const decide = async (decision: UseDecision) => {
        setBusy(true);
        try {
            await answerUse(u.id, decision);
            props.close();
        } catch (e) {
            const message = e instanceof Error ? e.message : String(e);
            // The agent gave up, or another window answered: nothing is waiting any more.
            if (/no longer waiting/i.test(message)) {
                props.close();
                return;
            }
            setError(message);
            setBusy(false);
        }
    };
    return (
        <Modal open={true} scope="window" onClose={() => void decide("deny")} size="sm" closeOnBackdropClick={false} closeOnEscape={!busy()}>
            <ModalHeader title={`${u.agent} wants to fill your ${u.what}`} />
            <ModalBody>
                <p>
                    Into <strong>{u.field}</strong> on <strong>{siteOf(u.origin)}</strong>
                    <span class="warden-manager-dim"> ({u.origin})</span>
                </p>
                <p>
                    From your Vault entry <strong>{u.entry_label}</strong>. You're asked because: {u.reason}.
                </p>
                <p class="warden-manager-dim">{"The agent never sees the value. Check the site is the one you expect."}</p>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss tone="danger" disabled={busy()} onClick={() => void decide("deny")}>
                    Deny
                </Button>
                <Button disabled={busy()} onClick={() => void decide("once")}>
                    Allow once
                </Button>
                <Show when={u.offer_always}>
                    <Button tone="attention" disabled={busy()} onClick={() => void decide("always")}>
                        Always on {siteOf(u.origin)}
                    </Button>
                </Show>
            </ModalFooter>
        </Modal>
    );
}

let started = false;

/** Follow srv's waiting uses and prompt for each new one. Call once. */
export function startVaultUsePrompts(): void {
    if (started || !vaultAvailable()) return;
    started = true;
    followVault();
    const shown = new Set<string>();
    createRoot(() =>
        createEffect(
            on(vaultUses, (list) => {
                const live = new Set(list.map((x) => x.id));
                for (const id of [...shown]) if (!live.has(id)) shown.delete(id);
                for (const u of list) {
                    if (shown.has(u.id)) continue;
                    shown.add(u.id);
                    modalsModel.openModal(UsePrompt, { use: u });
                }
            })
        )
    );
    // Uses that were waiting before this window started: needs no keychain read.
    void refreshVaultRequests();
}
