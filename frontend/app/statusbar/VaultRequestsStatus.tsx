// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openVault } from "@/app/view/vault/open-vault";
import { vaultRequests, vaultUses } from "@/app/view/vault/vault-store";
import { Show, type JSX } from "solid-js";

/** The tooltip of `n` Vault requests no pane on screen asks. */
export function vaultRequestsTip(n: number): string {
    return `${n} Vault ${n === 1 ? "request waits" : "requests wait"} on you with no agent pane on screen. Click to answer in the Vault`;
}

/**
 * How many Vault requests wait on the person with no agent pane on screen to ask
 * them (a parked agent's, or one whose window closed), and a click opens the
 * Vault pane's Requests list. A request asked as a card in its pane is counted by
 * the agents' "waiting on you" instead
 * (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §6).
 */
export function VaultRequestsStatus(): JSX.Element {
    const count = () => vaultUses().filter((u) => !u.block_id).length + vaultRequests().filter((r) => !r.block_id).length;
    return (
        <Show when={count() > 0}>
            <div
                class="status-bar-item clickable status-vault-requests"
                onClick={() => void openVault()}
                data-tip={vaultRequestsTip(count())}
                aria-label={vaultRequestsTip(count())}
                role="button"
                data-operator-only
            >
                <i class="fa-solid fa-vault status-icon" />
                <span style={{ color: "var(--warning-text-color)" }}>
                    {count()} Vault {count() === 1 ? "request" : "requests"}
                </span>
            </div>
        </Show>
    );
}

VaultRequestsStatus.displayName = "VaultRequestsStatus";
