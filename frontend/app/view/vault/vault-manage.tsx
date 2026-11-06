// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The person's Vaults: make one, name it, say which agents may use it, delete an empty one, and
// choose the Vault each browser profile keeps its sign-ins in
// (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §5.2, §5.3, §5.5).

import { createSignal, For, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, IconButton, SegmentedControl, TextInput } from "@/app/element/ui";
import type { VaultAgents } from "@/types/rpc/VaultAgents";
import type { VaultInfo } from "@/types/rpc/VaultInfo";
import { VaultSigninVaults } from "./vault-signin-vaults";
import { agentsSummary } from "./vault-vaults";
import { createVault, deleteVault, updateVault, vaultList } from "./vault-store";

/** Agent ids typed as a list: split at commas and spaces, empty ones dropped. */
export function parseAgentIds(text: string): string[] {
    return text
        .split(/[\s,;]+/)
        .map((s) => s.trim())
        .filter(Boolean);
}

type Mode = VaultAgents["mode"];

const MODES = [
    { value: "all", label: "All agents" },
    { value: "named", label: "Named agents" },
    { value: "none", label: "None" },
] satisfies { value: Mode; label: string }[];

/** A Vault's name and its agents; `vault` is the one being changed, none for a new Vault. */
function VaultEditor(props: { vault: VaultInfo | null; onDone: () => void }): JSX.Element {
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
            if (props.vault) await updateVault(props.vault.id, name(), agents());
            else await createVault(name(), agents());
            props.onDone();
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
            <Field label="Name" layout="stacked">
                <TextInput value={name()} onInput={(e) => setName(e.currentTarget.value)} maxLength={60} autofocus />
            </Field>
            <Field label="Agents that may use its entries" layout="stacked" description="An agent can't tell a Vault it may not use from one that doesn't exist. You can always use every entry yourself.">
                <SegmentedControl density="compact" ariaLabel="Agents" options={MODES} value={mode()} onChange={setMode} />
            </Field>
            <Show when={mode() === "named"}>
                <Field label="Agent ids" layout="stacked" description="Separated by commas, as the Swarm shows them (for example agent2-6).">
                    <TextInput value={ids()} onInput={(e) => setIds(e.currentTarget.value)} placeholder="agent2-6, builder" />
                </Field>
            </Show>
            <Show when={error()}>
                <div class="warden-manager-error" role="alert">
                    {error()}
                </div>
            </Show>
            <div class="vault-editor-actions">
                <Button density="compact" onClick={props.onDone} disabled={busy()}>
                    Cancel
                </Button>
                <Button density="compact" tone="accent" busy={busy()} onClick={() => void save()}>
                    {props.vault ? "Save" : "Add Vault"}
                </Button>
            </div>
        </form>
    );
}

function VaultRows(props: { onEdit: (v: VaultInfo) => void; onDelete: (v: VaultInfo) => void }): JSX.Element {
    return (
        <ul class="vault-vault-list">
            <For each={vaultList()}>
                {(v) => (
                    <li class="vault-vault-row" data-vault={v.id}>
                        <span class="vault-vault-name">{v.name}</span>
                        <span class="warden-manager-dim vault-vault-agents">
                            {agentsSummary(v.agents)} · {v.entries === 1 ? "1 entry" : `${v.entries} entries`}
                        </span>
                        <IconButton icon="pen" label={`Change ${v.name}`} density="compact" onClick={() => props.onEdit(v)} />
                        <Show when={v.id !== "personal"}>
                            <IconButton icon="trash" label={`Delete ${v.name}`} density="compact" onClick={() => props.onDelete(v)} />
                        </Show>
                    </li>
                )}
            </For>
        </ul>
    );
}

export function VaultManageDialog(props: { onClose: () => void }): JSX.Element {
    // `undefined`: the list; `null`: a new Vault; a Vault: that one being changed.
    const [editing, setEditing] = createSignal<VaultInfo | null | undefined>(undefined);
    const [deleting, setDeleting] = createSignal<VaultInfo | null>(null);
    const [error, setError] = createSignal<string | null>(null);

    const confirmDelete = async () => {
        const v = deleting();
        if (!v) return;
        try {
            await deleteVault(v.id);
            setError(null);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        setDeleting(null);
    };

    return (
        <Modal open={true} scope="pane" onClose={props.onClose} size="md" panelClass="vault-form">
            <ModalHeader title="Vaults" description="Each entry is in one Vault. A Vault says which agents may use its entries." />
            <ModalBody>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        ⚠ {error()}
                    </div>
                </Show>
                <Show when={editing() !== undefined} fallback={<VaultRows onEdit={setEditing} onDelete={setDeleting} />}>
                    <VaultEditor vault={editing() ?? null} onDone={() => setEditing(undefined)} />
                </Show>
                <Show when={editing() === undefined}>
                    <Button density="compact" icon="plus" onClick={() => setEditing(null)}>
                        Add a Vault
                    </Button>
                    <VaultSigninVaults />
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose}>
                    Close
                </Button>
            </ModalFooter>
            <Show when={deleting()}>
                {(v) => (
                    <ConfirmModal
                        open={true}
                        scope="pane"
                        title={`Delete ${v().name}?`}
                        description="A Vault can be deleted once it holds no entries: move them to another Vault, or delete them, first."
                        confirmLabel="Delete"
                        destructive
                        onConfirm={confirmDelete}
                        onCancel={() => setDeleting(null)}
                    />
                )}
            </Show>
        </Modal>
    );
}
