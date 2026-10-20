// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One Vault section's entries: label, kind, plain details, the main value's
// last characters, and Change / Delete. Values are never shown.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, FilterInput, IconButton } from "@/app/element/ui";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { VaultEntryForm, type VaultFormTarget } from "./vault-entry-form";
import { VaultRevealDialog } from "./vault-reveal";
import { formatWhen } from "./vault-format";
import { SECTION_LABELS, vaultKind } from "./vault-kinds";
import { deleteEntry, vaultEntries, vaultLoading } from "./vault-store";

import "@/app/view/warden-shared/warden-manager-chrome.scss";

/** The plain details worth a glance, in the order a person reads them. */
const DETAIL_FIELDS = ["owner", "service", "site", "username", "bank", "env_name", "issuer", "expires"];

export function entryDetails(e: VaultEntryInfo): string {
    return DETAIL_FIELDS.map((f) => e.fields[f])
        .filter(Boolean)
        .join(" · ");
}

export function entryMatches(e: VaultEntryInfo, filter: string): boolean {
    const q = filter.trim().toLowerCase();
    if (!q) return true;
    return [e.label, e.id, vaultKind(e.kind)?.label ?? e.kind, entryDetails(e), ...e.tags].some((s) => s.toLowerCase().includes(q));
}

export function VaultEntries(props: { section: VaultSection }): JSX.Element {
    const [filter, setFilter] = createSignal("");
    const [form, setForm] = createSignal<VaultFormTarget | null>(null);
    const [deleting, setDeleting] = createSignal<VaultEntryInfo | null>(null);
    const [revealing, setRevealing] = createSignal<VaultEntryInfo | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const rows = createMemo(() => (vaultEntries() ?? []).filter((e) => e.section === props.section && entryMatches(e, filter())));
    const total = createMemo(() => (vaultEntries() ?? []).filter((e) => e.section === props.section).length);

    const confirmDelete = async () => {
        const e = deleting();
        if (!e) return;
        try {
            await deleteEntry(e.id);
            setError(null);
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        }
        setDeleting(null);
    };

    return (
        <div class="warden-manager-body vault-entries">
            <div class="vault-bar">
                <Show when={total() > 0}>
                    <FilterInput value={filter()} onInput={setFilter} placeholder={`Filter ${SECTION_LABELS[props.section].toLowerCase()}`} />
                </Show>
                <Button tone="accent" density="compact" icon="plus" onClick={() => setForm({ mode: "new", section: props.section })}>
                    Add
                </Button>
            </div>
            <Show when={error()}>
                <div class="warden-manager-error" role="alert">
                    ⚠ {error()}
                </div>
            </Show>
            <Show
                when={rows().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {vaultEntries() === null && vaultLoading()
                            ? "Loading…"
                            : total() > 0
                              ? "No entry matches."
                              : "Nothing here yet. Add an entry, and agents can use it by name without seeing its value."}
                    </div>
                }
            >
                <table class="warden-manager-table vault-table">
                    <thead>
                        <tr>
                            <th>Label</th>
                            <th>Kind</th>
                            <th>Details</th>
                            <th>Value</th>
                            <th>Changed</th>
                            <th />
                        </tr>
                    </thead>
                    <tbody>
                        <For each={rows()}>
                            {(e) => (
                                <tr data-entry={e.id}>
                                    <td>
                                        <span>{e.label}</span>
                                        <span class="warden-manager-mono warden-manager-dim vault-id">{e.id}</span>
                                    </td>
                                    <td>{vaultKind(e.kind)?.label ?? e.kind}</td>
                                    <td class="warden-manager-dim">{entryDetails(e)}</td>
                                    <td class="warden-manager-mono">{e.tail ? `•••• ${e.tail}` : "••••"}</td>
                                    <td class="warden-manager-dim">{formatWhen(e.updated_at)}</td>
                                    <td class="vault-actions">
                                        <IconButton icon="eye" label="Show or copy" density="compact" onClick={() => setRevealing(e)} />
                                        <IconButton icon="pen" label="Change" density="compact" onClick={() => setForm({ mode: "edit", entry: e })} />
                                        <IconButton icon="trash" label="Delete" density="compact" onClick={() => setDeleting(e)} />
                                    </td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
            <Show when={form()}>{(target) => <VaultEntryForm target={target()} onClose={() => setForm(null)} />}</Show>
            <Show when={revealing()}>{(entry) => <VaultRevealDialog entry={entry()} onClose={() => setRevealing(null)} />}</Show>
            <Show when={deleting()}>
                {(e) => (
                    <ConfirmModal
                        open={true}
                        scope="pane"
                        title={`Delete ${e().label}?`}
                        description="Its value is deleted from this computer. Agents that use it by name will be told it's gone."
                        confirmLabel="Delete"
                        destructive
                        onConfirm={confirmDelete}
                        onCancel={() => setDeleting(null)}
                    />
                )}
            </Show>
        </div>
    );
}
