// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent's request for a new Vault entry, as a prompt for the person
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.4).
// "Fill it in" opens the Vault on the entry's form; the value is typed there,
// never here. "Not now" leaves the request in the Vault for later. The prompt
// shows only what the agent sent: its name, the kind, the label and why.

import { createEffect, createRoot, on, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { modalsModel, type ModalCloseProps } from "@/app/store/modalmodel";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import { vaultKind } from "./vault-kinds";
import { openVault } from "./open-vault";
import { startVaultUsePrompts } from "./vault-use-prompts";
import { followVault, refreshVaultRequests, vaultAvailable, vaultRequests } from "./vault-store";

function RequestPrompt(props: { request: VaultRequest } & ModalCloseProps): JSX.Element {
    const r = props.request;
    const kind = vaultKind(r.kind);
    // Answered elsewhere (in the Vault, another window): this prompt goes.
    createEffect(() => {
        if (!vaultRequests().some((x) => x.id === r.id)) props.close();
    });
    return (
        <ConfirmModal
            open={true}
            title={`${r.agent} asks for a Vault entry`}
            attention
            confirmLabel="Fill it in"
            cancelLabel="Not now"
            onConfirm={async () => {
                props.close();
                await openVault(kind?.section, r.id);
            }}
            onCancel={() => props.close()}
        >
            <p>
                {kind?.label ?? r.kind}: <strong>{r.label}</strong>
            </p>
            {r.why && <p>Why: {r.why}</p>}
            <p class="warden-manager-dim">{"The agent gets the entry's name to use, never its value."}</p>
        </ConfirmModal>
    );
}

let started = false;

/** Follow srv's Vault requests and prompt for each new one; the same for agents'
 *  uses of an entry. Call once. */
export function startVaultRequestPrompts(): void {
    if (started || !vaultAvailable()) return;
    started = true;
    startVaultUsePrompts();
    followVault();
    const shown = new Set<string>();
    createRoot(() =>
        createEffect(
            on(vaultRequests, (list) => {
                const live = new Set(list.map((r) => r.id));
                for (const id of [...shown]) if (!live.has(id)) shown.delete(id);
                for (const r of list) {
                    if (shown.has(r.id)) continue;
                    shown.add(r.id);
                    modalsModel.openModal(RequestPrompt, { request: r });
                }
            })
        )
    );
    // Requests made before this window started. Only the requests: reading
    // the entries could make the keychain ask the person at startup.
    void refreshVaultRequests();
}
