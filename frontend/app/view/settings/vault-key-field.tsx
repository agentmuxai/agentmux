// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A settings key that lives in the Vault: choose one of its API keys, or add one in the Vault's own form. The
// setting holds only a reference (`vault:<entry id>`); srv opens the entry when it uses the key, so the value is
// never written to settings.json or shown here.

import { Button, Notice, Select } from "@/app/element/ui";
import { createSignal, onMount, Show, type JSX } from "solid-js";

import { VaultEntryForm } from "@/app/view/vault/vault-entry-form";
import { refreshVault, vaultAvailable, vaultEntries } from "@/app/view/vault/vault-store";
import { entryIdOf, keyChoices, keyState, referenceTo } from "./vault-key-field-model";

export function VaultKeyField(p: {
    /** What the setting holds: a `vault:<id>` reference, a placeholder for a plain key, or nothing. */
    value: string | undefined;
    /** The person chose an entry: the reference to store as the setting. */
    onChoose: (reference: string) => void;
    /** What to call the key in the control's own words ("Groq key"). */
    what: string;
}): JSX.Element {
    const [adding, setAdding] = createSignal(false);
    onMount(() => {
        if (vaultAvailable()) void refreshVault();
    });
    const state = () => keyState(p.value, vaultEntries());
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
                    onChange={(id) => id && p.onChoose(referenceTo(id))}
                />
                <Button disabled={!vaultAvailable()} onClick={() => setAdding(true)}>
                    Add a key to the Vault…
                </Button>
            </div>
            <Show when={!vaultAvailable()}>
                <div class="setting-description">The Vault needs the AgentMux desktop app.</div>
            </Show>
            <Show when={state() === "gone"}>
                <Notice tone="warning">That Vault entry is not on this machine, or is not an API key. Choose another, or add one.</Notice>
            </Show>
            <Show when={state() === "plain"}>
                <Notice tone="warning">This key is still in settings.json as plain text. Choose or add a Vault key to replace it.</Notice>
            </Show>
            <Show when={adding()}>
                <VaultEntryForm
                    target={{ mode: "new", section: "api-keys" }}
                    onClose={(savedId) => {
                        setAdding(false);
                        if (savedId) p.onChoose(referenceTo(savedId));
                    }}
                />
            </Show>
        </div>
    );
}
