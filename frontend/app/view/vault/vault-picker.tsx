// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vaults row, at the top of the pane: the dropdown that narrows the list to one Vault, Change and Delete for
// the chosen one, and New Vault, whose form opens in the pane under the row
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.8).

import { createMemo, createSignal, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, IconButton, Select, type SelectOption } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultInfo } from "@/types/rpc/VaultInfo";
import { VaultManagePanel } from "./vault-manage";
import { deleteVault, vaultList } from "./vault-store";
import { agentsSummary, ALL_VAULTS } from "./vault-vaults";

/** What the Vaults row has open under it, and the Vault about to be deleted. */
function createVaultsRow(props: { value: string; onChange: (id: string) => void }) {
    // `undefined`: nothing; `null`: a new Vault's form; a Vault: that one's.
    const [editing, setEditing] = createSignal<VaultInfo | null | undefined>(undefined);
    const [deleting, setDeleting] = createSignal<VaultInfo | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    // The only Vault there is needs no choosing: it is the chosen one.
    const chosen = createMemo(() => vaultList().find((v) => v.id === props.value) ?? (vaultList().length === 1 ? vaultList()[0] : undefined));
    const options = createMemo<SelectOption[]>(() => [
        ...(vaultList().length > 1 ? [{ value: ALL_VAULTS, label: t("vault.vaults.all") }] : []),
        ...vaultList().map((v) => ({ value: v.id, label: `${v.name} (${v.entries})` })),
    ]);
    const done = (created?: string) => {
        setEditing(undefined);
        if (created) props.onChange(created);
    };
    const confirmDelete = async () => {
        const v = deleting();
        if (!v) return;
        try {
            await deleteVault(v.id);
            setError(null);
            setEditing(undefined);
            props.onChange(ALL_VAULTS);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        setDeleting(null);
    };
    return { editing, setEditing, deleting, setDeleting, error, chosen, options, done, confirmDelete };
}

/** The Vaults row: a dropdown that narrows the list to one Vault (or all of them), who the chosen Vault is for,
 *  Change and Delete for it, and New Vault. A form opens in the pane, under the row. */
export function VaultPicker(props: { value: string; onChange: (id: string) => void; extra?: JSX.Element }): JSX.Element {
    const s = createVaultsRow(props);
    return (
        <>
            <div class="vault-picker">
                <Select class="vault-picker-select" density="compact" ariaLabel={t("vault.vaults.choose")} options={s.options()} value={s.chosen()?.id ?? ALL_VAULTS} onChange={props.onChange} />
                <Show when={s.chosen()}>
                    {(v) => (
                        <>
                            <span class="warden-manager-dim vault-picker-agents">{agentsSummary(v().agents)}</span>
                            <IconButton icon="pen" label={t("vault.vaults.change", { name: v().name })} density="compact" onClick={() => s.setEditing(v())} />
                            <Show when={v().id !== "personal"}>
                                <IconButton icon="trash" label={t("vault.vaults.delete", { name: v().name })} density="compact" onClick={() => s.setDeleting(v())} />
                            </Show>
                        </>
                    )}
                </Show>
                <Button density="compact" icon="plus" onClick={() => s.setEditing(null)}>
                    {t("vault.vaults.new")}
                </Button>
                {props.extra}
            </div>
            <Show when={s.error()}>
                <div class="warden-manager-error vault-problem" role="alert">
                    {s.error()}
                </div>
            </Show>
            {/* Keyed by the Vault, so choosing Change on another Vault starts its form afresh. */}
            <Show when={s.editing() !== undefined ? { vault: s.editing() ?? null } : undefined} keyed>
                {(open) => <VaultManagePanel vault={open.vault} onDone={s.done} />}
            </Show>
            <Show when={s.deleting()}>
                {(v) => (
                    <ConfirmModal
                        open={true}
                        scope="pane"
                        title={t("vault.vaults.deleteTitle", { name: v().name })}
                        description={t("vault.vaults.deleteAbout")}
                        confirmLabel={t("vault.vaults.deleteConfirm")}
                        destructive
                        onConfirm={s.confirmDelete}
                        onCancel={() => s.setDeleting(null)}
                    />
                )}
            </Show>
        </>
    );
}
