// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One Vault section's entries: label, kind, plain details and expiry, the main
// value's last characters, and Change / Earlier values / Delete. Values are never shown.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, FilterInput, IconButton } from "@/app/element/ui";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { VaultEntryForm, type VaultFormTarget } from "./vault-entry-form";
import { VaultRevealDialog } from "./vault-reveal";
import { VaultVersionsDialog } from "./vault-versions";
import { EXPIRING_SOON_DAYS, expiryOf, formatWhen } from "./vault-format";
import { SECTION_LABELS, vaultKind } from "./vault-kinds";
import { ensureBrowserProfiles, profileById } from "@/app/view/browser/browser-profiles";
import { COMMAND_GRANT, commandGrantLabel, siteOf } from "./vault-use-prompts";
import { deleteEntry, revokeGrant, vaultEntries, vaultLoading } from "./vault-store";

import "@/app/view/warden-shared/warden-manager-chrome.scss";

/** The plain details worth a glance, in the order a person reads them. The expiry
 *  date has its own line (`expiryOf`). */
const DETAIL_FIELDS = ["owner", "service", "site", "realm", "username", "bank", "env_name", "issuer"];

/** A login's profile, when it isn't Personal: the tabs it is offered in. */
function profileDetail(e: VaultEntryInfo): string {
    const p = e.fields.profile;
    if (!p || p === "personal") return "";
    // The names load on first need; the line follows once they have.
    ensureBrowserProfiles();
    const id = p.replace(/^profile:/, "");
    return `profile ${profileById(id)?.name ?? "(deleted)"}`;
}

export function entryDetails(e: VaultEntryInfo): string {
    return [...DETAIL_FIELDS.map((f) => (f === "realm" && e.fields.proxy ? `proxy ${e.fields[f] ?? ""}`.trim() : e.fields[f])), profileDetail(e)]
        .filter(Boolean)
        .join(" · ");
}

export function entryMatches(e: VaultEntryInfo, filter: string): boolean {
    const q = filter.trim().toLowerCase();
    if (!q) return true;
    return [e.label, e.id, vaultKind(e.kind)?.label ?? e.kind, entryDetails(e), e.fields.expires ?? "", ...e.tags].some((s) => s.toLowerCase().includes(q));
}

/** Past its date or within `EXPIRING_SOON_DAYS` of it. */
export function expiringSoon(e: VaultEntryInfo, now = new Date()): boolean {
    const x = expiryOf(e.fields.expires, now);
    return x !== null && x.days <= EXPIRING_SOON_DAYS;
}

export function VaultEntries(props: { section: VaultSection }): JSX.Element {
    const [filter, setFilter] = createSignal("");
    const [soonOnly, setSoonOnly] = createSignal(false);
    const [form, setForm] = createSignal<VaultFormTarget | null>(null);
    const [deleting, setDeleting] = createSignal<VaultEntryInfo | null>(null);
    const [revealing, setRevealing] = createSignal<VaultEntryInfo | null>(null);
    const [versionsOf, setVersionsOf] = createSignal<VaultEntryInfo | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const inSection = createMemo(() => (vaultEntries() ?? []).filter((e) => e.section === props.section));
    const dated = createMemo(() => inSection().some((e) => expiryOf(e.fields.expires) !== null));
    const rows = createMemo(() => inSection().filter((e) => entryMatches(e, filter()) && (!soonOnly() || !dated() || expiringSoon(e))));
    const total = createMemo(() => inSection().length);

    const revoke = async (id: string, origin: string) => {
        try {
            await revokeGrant(id, origin);
            setError(null);
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        }
    };

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
                <Show when={dated()}>
                    <Button density="compact" icon="clock" pressed={soonOnly()} onClick={() => setSoonOnly(!soonOnly())}>
                        Expiring soon
                    </Button>
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
                                        <Show when={e.grants.some((g) => !g.startsWith(COMMAND_GRANT))}>
                                            <div class="vault-grants" aria-label={`Sites ${e.label} may be filled into without asking`}>
                                                <span class="warden-manager-dim">Agents may fill it on </span>
                                                <For each={e.grants.filter((g) => !g.startsWith(COMMAND_GRANT))}>
                                                    {(origin) => (
                                                        <span class="vault-grant" data-origin={origin}>
                                                            {siteOf(origin)}
                                                            <IconButton icon="xmark" label={`Stop filling ${e.label} on ${siteOf(origin)} without asking`} density="compact" onClick={() => void revoke(e.id, origin)} />
                                                        </span>
                                                    )}
                                                </For>
                                            </div>
                                        </Show>
                                        <Show when={e.grants.some((g) => g.startsWith(COMMAND_GRANT))}>
                                            <div class="vault-grants" aria-label={`Agents that may run commands with ${e.label} without asking`}>
                                                <span class="warden-manager-dim">Agents may run commands with it: </span>
                                                <For each={e.grants.filter((g) => g.startsWith(COMMAND_GRANT))}>
                                                    {(grant) => (
                                                        <span class="vault-grant" data-origin={grant}>
                                                            {commandGrantLabel(grant)}
                                                            <IconButton
                                                                icon="xmark"
                                                                label={`Stop ${commandGrantLabel(grant)} running commands with ${e.label} without asking`}
                                                                density="compact"
                                                                onClick={() => void revoke(e.id, grant)}
                                                            />
                                                        </span>
                                                    )}
                                                </For>
                                            </div>
                                        </Show>
                                    </td>
                                    <td>{vaultKind(e.kind)?.label ?? e.kind}</td>
                                    <td class="warden-manager-dim">
                                        {entryDetails(e)}
                                        <Show when={expiryOf(e.fields.expires)}>
                                            {(x) => (
                                                <div class={x().warn ? "vault-expiry vault-expiry-warn" : "vault-expiry"} title={e.fields.expires}>
                                                    {x().text}
                                                </div>
                                            )}
                                        </Show>
                                    </td>
                                    <td class="warden-manager-mono">{e.tail ? `•••• ${e.tail}` : "••••"}</td>
                                    <td class="warden-manager-dim">{formatWhen(e.updated_at)}</td>
                                    <td class="vault-actions">
                                        <IconButton icon="eye" label="Show or copy" density="compact" onClick={() => setRevealing(e)} />
                                        <IconButton icon="pen" label="Change" density="compact" onClick={() => setForm({ mode: "edit", entry: e })} />
                                        <Show when={e.versions > 0}>
                                            <IconButton icon="rotate-left" label="Earlier values" density="compact" onClick={() => setVersionsOf(e)} />
                                        </Show>
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
            <Show when={versionsOf()}>{(entry) => <VaultVersionsDialog entry={entry()} onClose={() => setVersionsOf(null)} />}</Show>
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
