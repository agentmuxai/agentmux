// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The person's Vaults, in the pane under the Vaults row (vault-picker.tsx): make one, name it, say which agents
// may use it, and choose the Vault each browser profile keeps its sign-ins in
// (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §5.2, §5.3, §5.5).

import { createSignal, Show, type JSX } from "solid-js";

import { Button, Field, SegmentedControl, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultAgents } from "@/types/rpc/VaultAgents";
import type { VaultInfo } from "@/types/rpc/VaultInfo";
import { VaultSigninVaults } from "./vault-signin-vaults";
import { createVault, updateVault } from "./vault-store";

/** Agent ids typed as a list: split at commas and spaces, empty ones dropped. */
export function parseAgentIds(text: string): string[] {
    return text
        .split(/[\s,;]+/)
        .map((s) => s.trim())
        .filter(Boolean);
}

type Mode = VaultAgents["mode"];

// Built when drawn, so a change of language shows.
const modes = () =>
    [
        { value: "all", label: t("vault.manage.modeAll") },
        { value: "named", label: t("vault.manage.modeNamed") },
        { value: "none", label: t("vault.manage.modeNone") },
    ] satisfies { value: Mode; label: string }[];

/** A Vault's name and its agents; `vault` is the one being changed, none for a new Vault. */
function VaultEditor(props: { vault: VaultInfo | null; onDone: (created?: string) => void }): JSX.Element {
    const now = props.vault?.agents;
    const [name, setName] = createSignal(props.vault?.name ?? "");
    const [mode, setMode] = createSignal<Mode>(now?.mode ?? "all");
    const [ids, setIds] = createSignal(now?.mode === "named" ? now.agents.join(", ") : "");
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const agents = (): VaultAgents => {
        const m = mode();
        return m === "named" ? { mode: m, agents: parseAgentIds(ids()) } : { mode: m };
    };
    const save = async () => {
        setBusy(true);
        try {
            if (props.vault) {
                await updateVault(props.vault.id, name(), agents());
                props.onDone();
            } else {
                props.onDone((await createVault(name(), agents()))?.id);
            }
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
            setBusy(false);
        }
    };
    return (
        <form
            class="vault-form-fields vault-editor"
            onSubmit={(e) => {
                e.preventDefault();
                void save();
            }}
        >
            <Field label={t("vault.manage.name")} layout="stacked">
                <TextInput value={name()} onInput={(e) => setName(e.currentTarget.value)} maxLength={60} autofocus />
            </Field>
            <Field label={t("vault.manage.agents")} layout="stacked" description={t("vault.manage.agentsHelp")}>
                <SegmentedControl density="compact" ariaLabel={t("vault.manage.agentsAria")} options={modes()} value={mode()} onChange={setMode} />
            </Field>
            <Show when={mode() === "named"}>
                <Field label={t("vault.manage.ids")} layout="stacked" description={t("vault.manage.idsHelp")}>
                    <TextInput value={ids()} onInput={(e) => setIds(e.currentTarget.value)} placeholder={t("vault.manage.idsExample")} />
                </Field>
            </Show>
            <Show when={error()}>
                <div class="warden-manager-error" role="alert">
                    {error()}
                </div>
            </Show>
            <div class="vault-editor-actions">
                <Button density="compact" onClick={() => props.onDone()} disabled={busy()}>
                    {t("vault.manage.cancel")}
                </Button>
                <Button density="compact" tone="accent" busy={busy()} onClick={() => save()}>
                    {props.vault ? t("vault.manage.save") : t("vault.manage.add")}
                </Button>
            </div>
        </form>
    );
}

/** In the pane, under the Vaults row: a new Vault's form (`vault` null), or the chosen Vault's, and with it the
 *  Vault each browser profile keeps its sign-ins in. `onDone` gets a new Vault's id. */
export function VaultManagePanel(props: { vault: VaultInfo | null; onDone: (created?: string) => void }): JSX.Element {
    const title = () => (props.vault ? t("vault.vaults.change", { name: props.vault.name }) : t("vault.vaults.new"));
    return (
        <section class="vault-manage" aria-label={title()}>
            <h4 class="vault-manage-title">{title()}</h4>
            <p class="warden-manager-dim">{t("vault.vaults.about")}</p>
            <VaultEditor vault={props.vault} onDone={props.onDone} />
            <Show when={props.vault}>
                <VaultSigninVaults />
            </Show>
        </section>
    );
}
