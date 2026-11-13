// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The person's Vaults in the pane: the picker that narrows the lists to one, which agents each is
// for, and Move to (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §5).

import { createMemo, createSignal, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, SegmentedControl, Select, type SegmentedOption, type SelectOption } from "@/app/element/ui";
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
    if (agents.mode === "all") return "All agents";
    if (agents.mode === "none") return "No agents: only you";
    return agents.agents.length === 0 ? "No agents: only you" : `Only ${agents.agents.join(", ")}`;
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
            <Field label={props.label ?? "Vault"} layout="stacked" description={props.description}>
                <Select value={props.value} onChange={props.onChange} options={vaultChoices()} ariaLabel={props.label ?? "Vault"} />
            </Field>
        </Show>
    );
}

/** Narrows the lists to one Vault, or all of them. Shown once there is more than one. */
export function VaultPicker(props: { value: string; onChange: (id: string) => void; onManage: () => void; extra?: JSX.Element }): JSX.Element {
    const options = createMemo<SegmentedOption<string>[]>(() => [
        { value: ALL_VAULTS, label: "All Vaults" },
        ...vaultList().map((v) => ({ value: v.id, label: `${v.name} (${v.entries})` })),
    ]);
    return (
        <div class="vault-picker">
            <Show when={vaultList().length > 1}>
                <SegmentedControl class="vault-picker-choices" density="compact" ariaLabel="Vault" options={options()} value={props.value} onChange={props.onChange} />
            </Show>
            {props.extra}
            <Button density="compact" icon="gear" onClick={props.onManage}>
                Vaults
            </Button>
        </div>
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
                title={`Move ${props.entry.label}`}
                description={`It is in ${vaultNameOf(props.entry.vault_id)} now. Its value, its earlier values and its permissions move with it.`}
            />
            <ModalBody>
                <Field label="Move to" layout="stacked" description={who() ? `Agents that may use it there: ${who()}.` : undefined}>
                    <Select value={target()} onChange={setTarget} options={others()} ariaLabel="Move to" />
                </Field>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose} disabled={busy()}>
                    Cancel
                </Button>
                <Button tone="accent" busy={busy()} disabled={!target()} onClick={() => void move()}>
                    Move
                </Button>
            </ModalFooter>
        </Modal>
    );
}
