// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One Vault section's entries: label, kind, plain details and expiry, the main
// value's last characters, and Change / Earlier values / Delete. Values are never shown.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, FilterInput, IconButton, Pill } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { VaultEntryForm, type VaultFormTarget } from "./vault-entry-form";
import { VaultHistoryDialog } from "./vault-history";
import { VaultRevealDialog } from "./vault-reveal";
import { VaultVersionsDialog } from "./vault-versions";
import { EXPIRING_SOON_DAYS, expiryOf, formatWhen } from "./vault-format";
import { CARD_BRANDS } from "./vault-card";
import { SECTION_LABELS, vaultKind } from "./vault-kinds";
import { machineCopy, MachineCopyLine } from "./vault-source";
import { CopyPlaceButton, isLinked, LinkedBadges, LinkedDetails } from "./vault-linked";
import { ensureBrowserProfiles, profileById } from "@/app/view/browser/browser-profiles";
import { EntryGrants } from "./vault-entry-grants";
import { checkSource, deleteEntry, revokeGrant, vaultEntries, vaultList, vaultLoading } from "./vault-store";
import { vaultNameOf, VaultMoveDialog } from "./vault-vaults";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import { tip } from "@/app/element/tip";

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
    const said = [e.fields.readers ?? "", e.fields.copies ?? ""];
    return [e.label, e.id, vaultKind(e.kind)?.label ?? e.kind, entryDetails(e), machineCopy(e.fields), ...said, e.fields.expires ?? "", ...e.tags].some((s) =>
        s.toLowerCase().includes(q),
    );
}

/** Past its date or within `EXPIRING_SOON_DAYS` of it. */
export function expiringSoon(e: VaultEntryInfo, now = new Date()): boolean {
    const x = expiryOf(e.fields.expires, now);
    return x !== null && x.days <= EXPIRING_SOON_DAYS;
}

/** Has srv read the entry's machine copy and compare it with the entry's value; for a linked entry, which holds no
 *  value, has it read the place's description. What it found is the entry's own line once srv has it; only a check
 *  that could not start is an error (`onError`). */
function CheckCopyButton(props: { entry: VaultEntryInfo; onError: (message: string | null) => void }): JSX.Element {
    const [busy, setBusy] = createSignal(false);
    const check = async () => {
        setBusy(true);
        try {
            await checkSource(props.entry.id);
            props.onError(null);
        } catch (err) {
            props.onError(err instanceof Error ? err.message : String(err));
        }
        setBusy(false);
    };
    const label = () => (isLinked(props.entry.fields) ? t("vault.linked.check") : t("vault.machineCopy.check"));
    return <IconButton icon="arrows-rotate" label={label()} density="compact" disabled={busy()} onClick={() => void check()} />;
}

/** An entry's plain details, the Vault it is in (when the list shows more than one), its expiry and its machine copy. */
function EntryDetails(props: { entry: VaultEntryInfo; showVault: boolean; onError: (message: string | null) => void }): JSX.Element {
    return (
        <>
            {entryDetails(props.entry)}
            <Show when={props.showVault && vaultList().length > 1}>
                <div class="vault-in-vault">in {vaultNameOf(props.entry.vault_id)}</div>
            </Show>
            <Show when={expiryOf(props.entry.fields.expires)}>
                {(x) => (
                    <div class={x().warn ? "vault-expiry vault-expiry-warn" : "vault-expiry"} {...tip(props.entry.fields.expires)}>
                        <Show when={props.entry.kind === "card" && x().days < 0}>
                            <Pill tone="error" icon="triangle-exclamation">
                                Expired
                            </Pill>{" "}
                        </Show>
                        {x().text}
                    </div>
                )}
            </Show>
            <MachineCopyLine entry={props.entry}>
                <CheckCopyButton entry={props.entry} onError={props.onError} />
            </MachineCopyLine>
            <LinkedDetails entry={props.entry} />
        </>
    );
}

/** An entry's label, a card's Default pill, and a linked entry's badges. */
function EntryLabel(props: { entry: VaultEntryInfo }): JSX.Element {
    return (
        <>
            <span>{props.entry.label}</span>
            <Show when={props.entry.kind === "card" && props.entry.fields.default === "yes"}>
                <Pill tone="accent" class="vault-default-pill">
                    Default
                </Pill>
            </Show>
            <LinkedBadges entry={props.entry} />
        </>
    );
}

/** The main value's last characters, and a card's brand before them. A linked entry has no value to show. */
function EntryValue(props: { entry: VaultEntryInfo }): JSX.Element {
    if (isLinked(props.entry.fields)) return <td class="warden-manager-dim vault-value">{t("vault.linked.noValue")}</td>;
    return (
        <td class="warden-manager-mono vault-value">
            <Show when={props.entry.kind === "card" && CARD_BRANDS[props.entry.fields.brand ?? ""]}>
                {(brand) => <Pill>{brand()}</Pill>}
            </Show>{" "}
            <span class="vault-last-four">{props.entry.tail ? `•••• ${props.entry.tail}` : "••••"}</span>
        </td>
    );
}

/** What a row offers: show or copy its value (a linked entry has none: its place is copied instead), change it, move
 *  it, read its history, put an earlier value back, delete it. */
function EntryActions(props: { entry: VaultEntryInfo; on: (what: "reveal" | "change" | "move" | "history" | "versions" | "delete") => void; onError: (message: string | null) => void }): JSX.Element {
    return (
        <td class="vault-actions">
            <Show when={!isLinked(props.entry.fields)} fallback={<CopyPlaceButton entry={props.entry} onError={props.onError} />}>
                <IconButton icon="eye" label="Show or copy" density="compact" onClick={() => props.on("reveal")} />
            </Show>
            <IconButton icon="pen" label="Change" density="compact" onClick={() => props.on("change")} />
            <Show when={vaultList().length > 1}>
                <IconButton icon="folder-open" label="Move to another Vault" density="compact" onClick={() => props.on("move")} />
            </Show>
            <IconButton icon="book" label="History" density="compact" onClick={() => props.on("history")} />
            <Show when={props.entry.versions > 0}>
                <IconButton icon="rotate-left" label="Earlier values" density="compact" onClick={() => props.on("versions")} />
            </Show>
            <IconButton icon="trash" label="Delete" density="compact" onClick={() => props.on("delete")} />
        </td>
    );
}

function DeleteEntryConfirm(props: { entry: VaultEntryInfo; onConfirm: () => Promise<void>; onCancel: () => void }): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title={`Delete ${props.entry.label}?`}
            description={
                // A link keeps no value: what goes is the record of where the key lives.
                isLinked(props.entry.fields)
                    ? t("vault.linked.deleteAbout")
                    : "Its value is deleted from this computer. Agents that use it by name will be told it's gone."
            }
            confirmLabel="Delete"
            destructive
            onConfirm={props.onConfirm}
            onCancel={props.onCancel}
        />
    );
}

/** `vault` narrows the list to one Vault (its id); empty shows every Vault's entries. */
export function VaultEntries(props: { section: VaultSection; vault?: string }): JSX.Element {
    const [filter, setFilter] = createSignal("");
    const [soonOnly, setSoonOnly] = createSignal(false);
    const [form, setForm] = createSignal<VaultFormTarget | null>(null);
    const [deleting, setDeleting] = createSignal<VaultEntryInfo | null>(null);
    const [revealing, setRevealing] = createSignal<VaultEntryInfo | null>(null);
    const [versionsOf, setVersionsOf] = createSignal<VaultEntryInfo | null>(null);
    const [moving, setMoving] = createSignal<VaultEntryInfo | null>(null);
    const [historyOf, setHistoryOf] = createSignal<VaultEntryInfo | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const inSection = createMemo(() => (vaultEntries() ?? []).filter((e) => e.section === props.section && (!props.vault || e.vault_id === props.vault)));
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

    const act = (what: "reveal" | "change" | "move" | "history" | "versions" | "delete", e: VaultEntryInfo) => {
        const open = { reveal: setRevealing, move: setMoving, history: setHistoryOf, versions: setVersionsOf, delete: setDeleting };
        if (what === "change") setForm({ mode: "edit", entry: e });
        else open[what](e);
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
                <Button tone="accent" density="compact" icon="plus" onClick={() => setForm({ mode: "new", section: props.section, vault: props.vault })}>
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
                                        <EntryLabel entry={e} />
                                        <span class="warden-manager-mono warden-manager-dim vault-id">{e.id}</span>
                                        <EntryGrants entry={e} revoke={(grant) => void revoke(e.id, grant)} />
                                    </td>
                                    <td>{vaultKind(e.kind)?.label ?? e.kind}</td>
                                    <td class="warden-manager-dim">
                                        <EntryDetails entry={e} showVault={!props.vault} onError={setError} />
                                    </td>
                                    <EntryValue entry={e} />
                                    <td class="warden-manager-dim vault-when">{formatWhen(e.updated_at)}</td>
                                    <EntryActions entry={e} on={(what) => act(what, e)} onError={setError} />
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
            <Show when={form()}>{(target) => <VaultEntryForm target={target()} onClose={() => setForm(null)} />}</Show>
            <Show when={revealing()}>{(entry) => <VaultRevealDialog entry={entry()} onClose={() => setRevealing(null)} />}</Show>
            <Show when={moving()}>{(entry) => <VaultMoveDialog entry={entry()} onClose={() => setMoving(null)} />}</Show>
            <Show when={versionsOf()}>{(entry) => <VaultVersionsDialog entry={entry()} onClose={() => setVersionsOf(null)} />}</Show>
            <Show when={historyOf()}>{(e) => <VaultHistoryDialog entryId={e().id} label={e().label} onClose={() => setHistoryOf(null)} />}</Show>
            <Show when={deleting()}>{(e) => <DeleteEntryConfirm entry={e()} onConfirm={confirmDelete} onCancel={() => setDeleting(null)} />}</Show>
        </div>
    );
}
