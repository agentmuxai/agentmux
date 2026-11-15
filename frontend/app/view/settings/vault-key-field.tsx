// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A settings key that lives in the Vault: choose one of its API keys, or add one in the Vault's own form. The
// setting holds only a reference (`vault:<entry id>`); srv opens the entry when it uses the key, so the value is
// never written to settings.json or shown here. Choosing also gives the Vault's grant (the entry may be read by
// this setting): a file that merely names an entry reads nothing.

import { Button, Notice, Select } from "@/app/element/ui";
import { createSignal, onMount, Show, type JSX } from "solid-js";

import { VaultEntryForm } from "@/app/view/vault/vault-entry-form";
import { allowSetting, refreshVault, vaultAvailable, vaultEntries } from "@/app/view/vault/vault-store";
import { entryIdOf, keyChoices, keyState, referenceTo } from "./vault-key-field-model";

export function VaultKeyField(p: {
    /** What the setting holds: a `vault:<id>` reference, a placeholder for a plain key, or nothing. */
    value: string | undefined;
    /** The settings key this field is for (`voice:groqApiKey`): the grant the Vault keeps is for this key. */
    setting: string;
    /** The person chose an entry, and the Vault has allowed the setting to read it: the reference to store. */
    onChoose: (reference: string) => void;
    /** What to call the key in the control's own words ("Groq key"). */
    what: string;
}): JSX.Element {
    const [adding, setAdding] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    /** The grant first, then the setting: a reference with no grant would read nothing. */
    const choose = async (id: string) => {
        try {
            await allowSetting(id, p.setting);
            // The list the field reads (the entry it was just added to, and its new grant) is the store's.
            await refreshVault();
            setError(null);
            p.onChoose(referenceTo(id));
        } catch (e) {
            setError(`Couldn't use that key for this setting: ${e instanceof Error ? e.message : e}`);
        }
    };
    onMount(() => {
        if (vaultAvailable()) void refreshVault();
    });
    const state = () => keyState(p.value, vaultEntries(), p.setting);
    const options = () => [
        { value: "", label: keyChoices(vaultEntries() ?? []).length ? `Choose a ${p.what} from the Vault…` : "No API keys in the Vault yet" },
        ...keyChoices(vaultEntries() ?? []).map((e) => ({ value: e.id, label: e.label })),
    ];
    return (
        <div class="setting-vault-key">
            <div class="setting-vault-key-controls">
                <Select
                    class="setting-select"
                    ariaLabel={`Vault entry for the ${p.what}`}
                    value={state() === "vault" ? (entryIdOf(p.value) ?? "") : ""}
                    options={options()}
                    disabled={!vaultAvailable()}
                    onChange={(id) => id && void choose(id)}
                />
                <Button disabled={!vaultAvailable()} onClick={() => setAdding(true)}>
                    Add a key to the Vault…
                </Button>
            </div>
            <Show when={error()}>
                <Notice tone="danger">{error()}</Notice>
            </Show>
            <Show when={!vaultAvailable()}>
                <div class="setting-description">The Vault needs the AgentMux desktop app.</div>
            </Show>
            <Show when={state() === "gone"}>
                <Notice tone="warning">That Vault entry is not on this machine, is not an API key, or was not allowed for this setting. Choose it again, or add one.</Notice>
            </Show>
            <Show when={state() === "plain"}>
                <Notice tone="warning">This key is still in settings.json as plain text. Choose or add a Vault key to replace it.</Notice>
            </Show>
            <Show when={adding()}>
                <VaultEntryForm
                    target={{ mode: "new", section: "api-keys" }}
                    onClose={(savedId) => {
                        setAdding(false);
                        if (savedId) void choose(savedId);
                    }}
                />
            </Show>
        </div>
    );
}
