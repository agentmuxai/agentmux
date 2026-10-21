// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The oversight half of the Vault, in Warden's Audit section
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.6): what
// agents did with the person's secrets, and where they may use them without asking,
// with a way to stop each. The entries themselves stay in the Vault pane, which this
// links to, and which links back.

import { createEffect, createSignal, For, on, onMount, Show, type JSX } from "solid-js";

import { Button, IconButton } from "@/app/element/ui";
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import { formatWhen } from "./vault-format";
import { openVault } from "./open-vault";
import { COMMAND_GRANT, commandGrantLabel, siteOf } from "./vault-use-prompts";
import { followVault, loadActivity, refreshVault, revokeGrant, vaultAvailable, vaultChangedMs, vaultEntries } from "./vault-store";

/** How many of an agent's recent uses to list. */
const RECENT = 50;

/** The person's own doings (saving, revealing, copying) are the Vault's to show; this is what agents did. */
export const isAgentActor = (a: VaultActivity): boolean => a.actor !== "you";

export function WardenSecrets(): JSX.Element {
    if (!vaultAvailable()) return <></>;
    const [rows, setRows] = createSignal<VaultActivity[] | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const load = () =>
        loadActivity(500)
            .then((r) => {
                setRows(r.filter(isAgentActor).slice(0, RECENT));
                setError(null);
            })
            .catch((e) => setError(String(e)));
    onMount(() => {
        followVault();
        void refreshVault();
    });
    // Again whenever the Vault changes (a use is recorded, a grant is made or revoked).
    createEffect(on(vaultChangedMs, () => void load()));

    const granted = () => (vaultEntries() ?? []).filter((e) => e.grants.length > 0);
    const revoke = async (entryId: string, origin: string) => {
        try {
            await revokeGrant(entryId, origin);
            await refreshVault();
        } catch (e) {
            setError(String(e));
        }
    };
    const grantText = (g: string) => (g.startsWith(COMMAND_GRANT) ? `runs commands: ${commandGrantLabel(g)}` : siteOf(g));

    return (
        <section class="warden-secrets" aria-label="Secrets">
            <h3 class="warden-manager-summary">Secrets: what agents did with them</h3>
            <p class="warden-manager-dim">
                Every use of a Vault entry by an agent, and where each may use one without asking. Never a value.{" "}
                <Button density="compact" onClick={() => void openVault("activity")}>
                    Open the Vault
                </Button>
            </p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show when={granted().length > 0}>
                <table class="warden-manager-table" aria-label="Standing permissions">
                    <thead>
                        <tr>
                            <th>Entry</th>
                            <th>May be used without asking</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={granted()}>
                            {(e) => (
                                <tr data-entry={e.id}>
                                    <td>{e.label}</td>
                                    <td>
                                        <For each={e.grants}>
                                            {(g) => (
                                                <span class="vault-grant" data-origin={g}>
                                                    {grantText(g)}
                                                    <IconButton icon="xmark" label={`Stop ${grantText(g)} for ${e.label}`} density="compact" onClick={() => void revoke(e.id, g)} />
                                                </span>
                                            )}
                                        </For>
                                    </td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
            <Show when={(rows() ?? []).length > 0} fallback={<div class="warden-section-stub">{rows() === null ? "Loading…" : "No agent has used a secret yet."}</div>}>
                <table class="warden-manager-table" aria-label="Agents' use of secrets">
                    <thead>
                        <tr>
                            <th>When</th>
                            <th>Agent</th>
                            <th>What</th>
                            <th>Entry</th>
                            <th>Where</th>
                            <th>Outcome</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={rows()}>
                            {(a) => (
                                <tr data-outcome={a.outcome}>
                                    <td class="warden-manager-dim">{formatWhen(a.at)}</td>
                                    <td class="warden-manager-mono">{a.actor}</td>
                                    <td>{a.action}</td>
                                    <td>{a.entry_label}</td>
                                    <td class="warden-manager-dim">{a.target}</td>
                                    <td>{a.outcome}</td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
        </section>
    );
}
