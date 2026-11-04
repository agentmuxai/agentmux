// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent's use of a Vault entry, as a prompt in the window, for a use that has
// no agent pane to ask in: a use from an agent with a pane is asked there, as a
// card (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4).
// The question is `UseQuestion`'s; closing the prompt denies. Also how a grant
// reads, for the Vault pane.

import { createEffect, createRoot, on, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { modalsModel, type ModalCloseProps } from "@/app/store/modalmodel";
import type { UseRequest } from "@/types/rpc/UseRequest";
import { followVault, refreshVaultRequests, vaultAvailable, vaultUses } from "./vault-store";
import { createUseAnswer, promptTitle, siteOf, UseQuestion } from "./vault-use-question";

export { clock, siteOf } from "./vault-use-question";

/** A grant for commands is stored as `command:<agent>:<part>`; a site's is its origin. */
export const COMMAND_GRANT = "command:";

/** An agent's "Always" for signing in with a saved sign-in is stored as
 *  `signin:<agent>|<profile>|<origin>`: the site and profile the person approved. */
export const SIGNIN_GRANT = "signin:";

/** `signin:<agent>|<profile>|<origin>` as a person reads it: `Agent1 on example.com`. */
export function signinGrantLabel(grant: string): string {
    const [agent, , origin] = grant.slice(SIGNIN_GRANT.length).split("|");
    return origin ? `${agent} on ${siteOf(origin)}` : agent;
}

/** A grant for a site, not one for an agent's commands or sign-ins. */
export function isSiteGrant(grant: string): boolean {
    return !grant.startsWith(COMMAND_GRANT) && !grant.startsWith(SIGNIN_GRANT);
}

/** `command:<agent>:<part>` as a person reads it: `Agent1 (value)`. */
export function commandGrantLabel(grant: string): string {
    const rest = grant.slice(COMMAND_GRANT.length);
    const cut = rest.lastIndexOf(":");
    return cut < 0 ? rest : `${rest.slice(0, cut)} (${rest.slice(cut + 1)})`;
}

/** A grant as a person reads it, in the list of what an agent may do without asking. */
export function grantText(g: string): string {
    if (g.startsWith(COMMAND_GRANT)) return `runs commands: ${commandGrantLabel(g)}`;
    return g.startsWith(SIGNIN_GRANT) ? `signs in: ${signinGrantLabel(g)}` : siteOf(g);
}

export function UsePrompt(props: { use: UseRequest } & ModalCloseProps): JSX.Element {
    const answer = createUseAnswer(props.use, () => props.close());
    return (
        <Modal open={true} scope="window" operatorOnly onClose={() => void answer.decide("deny")} size="sm" closeOnBackdropClick={false} closeOnEscape={!answer.busy()}>
            <ModalHeader title={promptTitle(props.use, true)} />
            <UseQuestion use={props.use} answer={answer} Body={ModalBody} Actions={ModalFooter} focus />
        </Modal>
    );
}

/** Whether a use is asked as a card in its agent's pane rather than here. */
export const askedInPane = (u: { block_id: string }): boolean => !!u.block_id;

let started = false;

/** Follow srv's waiting uses and prompt for each new one an agent's pane
 *  doesn't ask. Call once. */
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
                    if (shown.has(u.id) || askedInPane(u)) continue;
                    shown.add(u.id);
                    modalsModel.openModal(UsePrompt, { use: u });
                }
            })
        )
    );
    // Uses that were waiting before this window started: needs no keychain read.
    void refreshVaultRequests();
}
