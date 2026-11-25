// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The person's Vaults in the pane: their names and choices, which agents each is for, and Move to. The Vaults
// row is vault-picker.tsx
// (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §5).

import { createSignal, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, Select, type SelectOption } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultAgents } from "@/types/rpc/VaultAgents";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { moveEntry, vaultList } from "./vault-store";

/** The picker's value for "every Vault". */
export const ALL_VAULTS = "";

/** A Vault's name by id; the id itself for one that is gone. */
export function vaultNameOf(id: string): string {
    return vaultList().find((v) => v.id === id)?.name ?? id;
}

/** Which agents a Vault is for, in a few words. */
export function agentsSummary(agents: VaultAgents): string {
    if (agents.mode === "all") return t("vault.vaults.agentsAll");
    if (agents.mode === "none") return t("vault.vaults.agentsNone");
    return agents.agents.length === 0 ? t("vault.vaults.agentsNone") : t("vault.vaults.agentsOnly", { agents: agents.agents.join(", ") });
}

/** The Vaults as choices for a Select. */
export function vaultChoices(except?: string): SelectOption[] {
    return vaultList()
        .filter((v) => v.id !== except)
        .map((v) => ({ value: v.id, label: v.name }));
}

/** The Vault to put something in, asked only when there is more than one. */
export function VaultField(props: { value: string; onChange: (id: string) => void; label?: string; description?: string }): JSX.Element {
    return (
        <Show when={vaultList().length > 1}>
            <Field label={props.label ?? t("vault.vaults.choose")} layout="stacked" description={props.description}>
                <Select value={props.value} onChange={props.onChange} options={vaultChoices()} ariaLabel={props.label ?? t("vault.vaults.choose")} />
            </Field>
        </Show>
    );
}

/** Moves an entry to another Vault. A move can open its value to more agents, so it says who they are first. */
export function VaultMoveDialog(props: { entry: VaultEntryInfo; onClose: () => void }): JSX.Element {
    const others = () => vaultChoices(props.entry.vault_id);
    const [target, setTarget] = createSignal(others()[0]?.value ?? "");
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const who = () => {
        const v = vaultList().find((x) => x.id === target());
        return v ? agentsSummary(v.agents) : "";
    };
    const move = async () => {
        setBusy(true);
        try {
            await moveEntry(props.entry.id, target());
            props.onClose();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
            setBusy(false);
        }
    };
    return (
        <Modal open={true} scope="pane" onClose={props.onClose} size="md" panelClass="vault-form">
            <ModalHeader
                title={t("vault.vaults.moveTitle", { label: props.entry.label })}
                description={t("vault.vaults.moveAbout", { vault: vaultNameOf(props.entry.vault_id) })}
            />
            <ModalBody>
                <Field label={t("vault.vaults.moveTo")} layout="stacked" description={who() ? t("vault.vaults.moveAgents", { who: who() }) : undefined}>
                    <Select value={target()} onChange={setTarget} options={others()} ariaLabel={t("vault.vaults.moveTo")} />
                </Field>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose} disabled={busy()}>
                    {t("vault.vaults.cancel")}
                </Button>
                <Button tone="accent" busy={busy()} disabled={!target()} onClick={() => move()}>
                    {t("vault.vaults.move")}
                </Button>
            </ModalFooter>
        </Modal>
    );
}
