// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault each browser profile saves its sign-ins to and offers them from
// (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §5.3).

import { createSignal, For, Show, type JSX } from "solid-js";

import { Field, Select } from "@/app/element/ui";
import { browserProfiles, ensureBrowserProfiles } from "@/app/view/browser/browser-profiles";
import { setSigninVault, signinVaultOf, vaultList } from "./vault-store";
import { vaultChoices } from "./vault-vaults";

/** The profiles that keep sign-ins: Personal, then the person's own. Incognito never saves one. */
export function profileRows(): { key: string; label: string }[] {
    const named = browserProfiles()
        .filter((p) => !p.owner_agent)
        .map((p) => ({ key: `profile:${p.id}`, label: p.name }));
    return [{ key: "personal", label: "Personal" }, ...named];
}

export function VaultSigninVaults(): JSX.Element {
    ensureBrowserProfiles();
    const [error, setError] = createSignal<string | null>(null);
    const choose = async (profile: string, vaultId: string) => {
        try {
            await setSigninVault(profile, vaultId);
            setError(null);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <Show when={vaultList().length > 1}>
            <section class="vault-signin-vaults" aria-label="Vault for browser sign-ins">
                <h4>Vault for browser sign-ins</h4>
                <p class="warden-manager-dim">
                    A browser profile saves new sign-ins in its Vault, and its key button offers the sign-ins kept there. Sign-ins already saved stay where they are:
                    move them to the profile's Vault to keep them offered. Incognito always uses Personal.
                </p>
                <For each={profileRows()}>
                    {(p) => (
                        <Field label={p.label} layout="inline">
                            <Select
                                value={signinVaultOf()[p.key] ?? "personal"}
                                onChange={(v) => void choose(p.key, v)}
                                options={vaultChoices()}
                                ariaLabel={`Vault for ${p.label} sign-ins`}
                            />
                        </Field>
                    )}
                </For>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </section>
        </Show>
    );
}
