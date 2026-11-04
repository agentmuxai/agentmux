// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A parked agent's Vault requests on its My agents row
 * (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §6): no
 * pane shows them, so the row says what the agent waits on, with srv's
 * countdown, Deny, and Answer, which opens the Vault pane's Requests list.
 */

import { Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { openVault } from "@/app/view/vault/open-vault";
import { answerUse, declineRequest, vaultRequests, vaultUses } from "@/app/view/vault/vault-store";
import { CLIP, Clipped, clock, secondsLeft } from "@/app/view/vault/vault-use-question";
import type { UseRequest } from "@/types/rpc/UseRequest";
import "./vault-request-card.scss";

function Countdown(props: { use: UseRequest }): JSX.Element {
    const left = secondsLeft(props.use);
    return <Show when={props.use.goes_ahead}> · goes ahead in {clock(left())}</Show>;
}

/** What `agent` (its id) waits on with no pane to ask in, if anything; nothing
 *  without an agent (its row has a pane, which asks its requests). */
export function VaultRowAsks(props: { agent?: string }): JSX.Element {
    const mine = (a: string) => !!props.agent && a.toLowerCase() === props.agent.toLowerCase();
    const uses = () => vaultUses().filter((u) => !u.block_id && mine(u.agent));
    const asks = () => vaultRequests().filter((r) => !r.block_id && mine(r.agent));
    const count = () => uses().length + asks().length;
    const first = () => uses()[0]?.entry_label ?? asks()[0]?.label ?? "";
    // Only what this line shows: a request asked in a pane on screen is answered there.
    const deny = () => Promise.all([...uses().map((u) => answerUse(u.id, "deny").catch(() => {})), ...asks().map((r) => declineRequest(r.id).catch(() => {}))]);
    return (
        <Show when={count() > 0}>
            <span class="vault-row-asks" data-operator-only data-testid="vault-row-asks">
                <span class="vault-row-asks-text">
                    Waiting for you in the Vault: <Clipped text={first()} max={CLIP.label} />
                    {count() > 1 ? ` and ${count() - 1} more` : ""}
                    <Show when={uses()[0]}>{(u) => <Countdown use={u()} />}</Show>
                </span>
                <Button tone="danger" density="compact" onClick={() => void deny()}>
                    {count() > 1 ? "Deny all" : "Deny"}
                </Button>
                <Button tone="attention" density="compact" onClick={() => void openVault()}>
                    Answer
                </Button>
            </span>
        </Show>
    );
}
