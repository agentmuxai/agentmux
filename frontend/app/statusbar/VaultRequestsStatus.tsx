// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openVaultAt } from "@/app/view/vault/open-vault";
import { vaultRequests, vaultUses } from "@/app/view/vault/vault-store";
import { Show, type JSX } from "solid-js";

/** The tooltip of `n` Vault requests that wait on the person. */
export function vaultRequestsTip(n: number): string {
    return `${n} Vault ${n === 1 ? "request waits" : "requests wait"} on you. Click to answer in the Vault`;
}

/**
 * How many Vault requests wait on the person, whatever pane asked: every use and every request the Vault holds
 * (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §3). One asked as a card in a pane used to
 * be counted only as that agent waiting, and one whose pane had closed by neither, so it could wait unseen. A pane that
 * waits only on a Vault request is not also counted as an agent waiting (`isVaultOnly`). A click opens the Vault pane's
 * Requests list at the first.
 */
export function VaultRequestsStatus(): JSX.Element {
    const unasked = () => [...vaultUses(), ...vaultRequests()];
    const count = () => unasked().length;
    return (
        <Show when={count() > 0}>
            <div
                class="status-bar-item clickable status-vault-requests"
                onClick={() => void openVaultAt(unasked()[0]?.id)}
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
