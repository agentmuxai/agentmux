// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One section of the Vault's list (vault-list.tsx): the entries of one type. A row has the label, the kind,
// the plain details and expiry, the main value's last characters, and Change / Earlier values / Delete.
// Values are never shown.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, IconButton, Pill } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { VaultEntryForm, type VaultFormTarget } from "./vault-entry-form";
import { VaultHistoryDialog } from "./vault-history";
import { VaultRevealDialog } from "./vault-reveal";
import { VaultVersionsDialog } from "./vault-versions";
import { EXPIRING_SOON_DAYS, expiryOf, formatWhen } from "./vault-format";
import { CARD_BRANDS } from "./vault-card";
import { kindsIn, SECTION_LABELS, vaultKind } from "./vault-kinds";
import { copyCheck, machineCopy, MachineCopyLine } from "./vault-source";
import { CopyPlaceButton, isBacked, isLinked, LinkedBadges, LinkedDetails } from "./vault-linked";
import { ensureBrowserProfiles, profileById } from "@/app/view/browser/browser-profiles";
import { EntryGrants } from "./vault-entry-grants";
import { canShare, ShareButton } from "./vault-share";
import { checkSource, deleteEntry, revokeGrant, vaultEntries, vaultList } from "./vault-store";
import { VaultSectionBox } from "./vault-section";
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
    return t("vault.entries.profile", { name: profileById(id)?.name ?? t("vault.entries.deleted") });
}

export function entryDetails(e: VaultEntryInfo): string {
    return [...DETAIL_FIELDS.map((f) => (f === "realm" && e.fields.proxy ? t("vault.entries.proxy", { realm: e.fields[f] ?? "" }).trim() : e.fields[f])), profileDetail(e)]
        .filter(Boolean)
        .join(" · ");
}

export function entryMatches(e: VaultEntryInfo, filter: string): boolean {
    const q = filter.trim().toLowerCase();
    if (!q) return true;
    const said = [e.fields.readers ?? "", e.fields.copies ?? ""];
    const type = [vaultKind(e.kind)?.label ?? e.kind, SECTION_LABELS[e.section] ?? ""];
    return [e.label, e.id, ...type, entryDetails(e), machineCopy(e.fields), ...said, e.fields.expires ?? "", ...e.tags].some((s) =>
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
                <div class="vault-in-vault">{t("vault.entries.inVault", { vault: vaultNameOf(props.entry.vault_id) })}</div>
            </Show>
            <Show when={expiryOf(props.entry.fields.expires)}>
                {(x) => (
                    <div class={x().warn ? "vault-expiry vault-expiry-warn" : "vault-expiry"} {...tip(props.entry.fields.expires)}>
                        <Show when={props.entry.kind === "card" && x().days < 0}>
                            <Pill tone="error" icon="triangle-exclamation">
                                {t("vault.entries.expired")}
                            </Pill>{" "}
                        </Show>
                        {x().text}
                    </div>
                )}
            </Show>
            <MachineCopyLine entry={props.entry}>
                <CheckCopyButton entry={props.entry} onError={props.onError} />
            </MachineCopyLine>
            <LinkedDetails entry={props.entry} copyNote={(line) => copyCheck(props.entry, line)} />
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
                    {t("vault.entries.default")}
                </Pill>
            </Show>
            <LinkedBadges entry={props.entry} />
        </>
    );
}

/** The main value's last characters, and a card's brand before them. A linked entry has no value to show. */
function EntryValue(props: { entry: VaultEntryInfo }): JSX.Element {
    if (isLinked(props.entry.fields)) return <td class="warden-manager-dim vault-value">{isBacked(props.entry.fields) ? t("vault.backed.noValue") : t("vault.linked.noValue")}</td>;
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
                <IconButton icon="eye" label={t("vault.entries.showCopy")} density="compact" onClick={() => props.on("reveal")} />
            </Show>
            <IconButton icon="pen" label={t("vault.entries.change")} density="compact" onClick={() => props.on("change")} />
            <Show when={vaultList().length > 1}>
                <IconButton icon="folder-open" label={t("vault.entries.move")} density="compact" onClick={() => props.on("move")} />
            </Show>
            <Show when={canShare(props.entry)}>
                <ShareButton entry={props.entry} />
            </Show>
            <IconButton icon="book" label={t("vault.entries.history")} density="compact" onClick={() => props.on("history")} />
            <Show when={props.entry.versions > 0}>
                <IconButton icon="rotate-left" label={t("vault.entries.earlier")} density="compact" onClick={() => props.on("versions")} />
            </Show>
            <IconButton icon="trash" label={t("vault.entries.delete")} density="compact" onClick={() => props.on("delete")} />
        </td>
    );
}

function DeleteEntryConfirm(props: { entry: VaultEntryInfo; onConfirm: () => Promise<void>; onCancel: () => void }): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title={t("vault.entries.deleteTitle", { label: props.entry.label })}
            description={
                // A link keeps no value: what goes is the record of where the key lives.
                isLinked(props.entry.fields)
                    ? t("vault.linked.deleteAbout")
                    : t("vault.entries.deleteAbout")
            }
            confirmLabel={t("vault.entries.delete")}
            destructive
            onConfirm={props.onConfirm}
            onCancel={props.onCancel}
        />
    );
}


/** The entries of one section, in one Vault (its id) or, with none named, in all of them. */
export function entriesOf(all: VaultEntryInfo[] | null, section: VaultSection, vault?: string): VaultEntryInfo[] {
    return (all ?? []).filter((e) => e.section === section && (!vault || e.vault_id === vault));
}

/** Whether the list is narrowed: a filter is typed, or only what expires soon is asked for. */
export const isNarrowing = (filter: string, soonOnly: boolean): boolean => filter.trim() !== "" || soonOnly;

/** What a narrowed list shows of `entries`. */
export function narrowedEntries(entries: VaultEntryInfo[], filter: string, soonOnly: boolean): VaultEntryInfo[] {
    return entries.filter((e) => entryMatches(e, filter) && (!soonOnly || expiringSoon(e)));
}

type EntryAction = "reveal" | "change" | "move" | "history" | "versions" | "delete";

export interface VaultEntriesProps {
    section: VaultSection;
    /** Narrows the section to one Vault (its id); empty shows every Vault's entries. */
    vault?: string;
    /** The list's one filter and its Expiring soon switch (vault-list.tsx): the section shows what matches. */
    filter?: string;
    soonOnly?: boolean;
    /** Folded when false. A narrowed list shows what it found, folded or not. */
    open?: boolean;
    onToggle?: () => void;
}

/** What one section's rows and dialogs share. */
function createEntriesState(props: VaultEntriesProps) {
    const [form, setForm] = createSignal<VaultFormTarget | null>(null);
    const [deleting, setDeleting] = createSignal<VaultEntryInfo | null>(null);
    const [revealing, setRevealing] = createSignal<VaultEntryInfo | null>(null);
    const [versionsOf, setVersionsOf] = createSignal<VaultEntryInfo | null>(null);
    const [moving, setMoving] = createSignal<VaultEntryInfo | null>(null);
    const [historyOf, setHistoryOf] = createSignal<VaultEntryInfo | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const inSection = createMemo(() => entriesOf(vaultEntries(), props.section, props.vault));
    const narrowing = () => isNarrowing(props.filter ?? "", !!props.soonOnly);
    const rows = createMemo(() => narrowedEntries(inSection(), props.filter ?? "", !!props.soonOnly));
    const total = () => inSection().length;
    const count = () => (narrowing() ? t("vault.section.countOf", { shown: rows().length, total: total() }) : String(total()));

    const guarded = async (run: () => Promise<unknown>) => {
        try {
            await run();
            setError(null);
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        }
    };
    const revoke = (id: string, origin: string) => void guarded(() => revokeGrant(id, origin));
    const act = (what: EntryAction, e: VaultEntryInfo) => {
        const open = { reveal: setRevealing, move: setMoving, history: setHistoryOf, versions: setVersionsOf, delete: setDeleting };
        if (what === "change") setForm({ mode: "edit", entry: e });
        else open[what](e);
    };
    const confirmDelete = async () => {
        const e = deleting();
        if (e) await guarded(() => deleteEntry(e.id));
        setDeleting(null);
    };
    const add = () => setForm({ mode: "new", section: props.section, vault: props.vault });
    return { form, setForm, deleting, setDeleting, revealing, setRevealing, versionsOf, setVersionsOf, moving, setMoving, historyOf, setHistoryOf, error, setError, narrowing, rows, count, revoke, act, confirmDelete, add };
}

type EntriesState = ReturnType<typeof createEntriesState>;

/** A section's rows. The columns are named for a screen reader only: each section would repeat them, and a row
 *  reads without them. A section of one kind has no Kind column, since its name says it. */
function EntryTable(props: { s: EntriesState; section: VaultSection; showVault: boolean }): JSX.Element {
    const s = props.s;
    const kinds = () => kindsIn(props.section).length > 1;
    return (
        <table class="warden-manager-table vault-table vault-entry-table">
            {/* In a wide pane these give every section the same column widths (vault-view.scss). */}
            <colgroup>
                <col class="vault-col-label" />
                <Show when={kinds()}>
                    <col class="vault-col-kind" />
                </Show>
                <col />
                <col class="vault-col-value" />
                <col class="vault-col-when" />
                <col class="vault-col-actions" />
            </colgroup>
            <thead class="vault-entry-columns">
                <tr>
                    <th>{t("vault.entries.colLabel")}</th>
                    <Show when={kinds()}>
                        <th>{t("vault.entries.colKind")}</th>
                    </Show>
                    <th>{t("vault.entries.colDetails")}</th>
                    <th>{t("vault.entries.colValue")}</th>
                    <th>{t("vault.entries.colChanged")}</th>
                    <th />
                </tr>
            </thead>
            <tbody>
                <For each={s.rows()}>
                    {(e) => (
                        <tr data-entry={e.id}>
                            <td>
                                <EntryLabel entry={e} />
                                <span class="warden-manager-mono warden-manager-dim vault-id" {...tip(e.id)}>
                                    {e.id}
                                </span>
                                <EntryGrants entry={e} revoke={(grant) => s.revoke(e.id, grant)} />
                            </td>
                            <Show when={kinds()}>
                                <td class="vault-kind">{vaultKind(e.kind)?.label ?? e.kind}</td>
                            </Show>
                            <td class="warden-manager-dim vault-details">
                                <EntryDetails entry={e} showVault={props.showVault} onError={s.setError} />
                            </td>
                            <EntryValue entry={e} />
                            <td class="warden-manager-dim vault-when">{formatWhen(e.updated_at)}</td>
                            <EntryActions entry={e} on={(what) => s.act(what, e)} onError={s.setError} />
                        </tr>
                    )}
                </For>
            </tbody>
        </table>
    );
}

/** The dialogs a row opens. They stay mounted when a narrowed list hides the section they were opened from. */
function EntryDialogs(props: { s: EntriesState }): JSX.Element {
    const s = props.s;
    return (
        <>
            <Show when={s.form()}>{(target) => <VaultEntryForm target={target()} onClose={() => s.setForm(null)} />}</Show>
            <Show when={s.revealing()}>{(entry) => <VaultRevealDialog entry={entry()} onClose={() => s.setRevealing(null)} />}</Show>
            <Show when={s.moving()}>{(entry) => <VaultMoveDialog entry={entry()} onClose={() => s.setMoving(null)} />}</Show>
            <Show when={s.versionsOf()}>{(entry) => <VaultVersionsDialog entry={entry()} onClose={() => s.setVersionsOf(null)} />}</Show>
            <Show when={s.historyOf()}>{(e) => <VaultHistoryDialog entryId={e().id} label={e().label} onClose={() => s.setHistoryOf(null)} />}</Show>
            <Show when={s.deleting()}>{(e) => <DeleteEntryConfirm entry={e()} onConfirm={s.confirmDelete} onCancel={() => s.setDeleting(null)} />}</Show>
        </>
    );
}

/** One section of the Vault's list: the entries of one type. With nothing in it, it is its head alone; in a
 *  narrowed list a section with no match is not shown. */
export function VaultEntries(props: VaultEntriesProps): JSX.Element {
    const s = createEntriesState(props);
    const label = () => SECTION_LABELS[props.section];
    return (
        <>
            <Show when={!s.narrowing() || s.rows().length > 0}>
                <VaultSectionBox
                    id={props.section}
                    count={s.count()}
                    open={s.narrowing() || (props.open ?? true)}
                    onToggle={() => props.onToggle?.()}
                    empty={s.rows().length === 0}
                    actions={
                        <Button density="compact" icon="plus" aria-label={t("vault.section.addTo", { section: label() })} onClick={s.add}>
                            {t("vault.section.add")}
                        </Button>
                    }
                >
                    <Show when={s.error()}>
                        <div class="warden-manager-error" role="alert">
                            ⚠ {s.error()}
                        </div>
                    </Show>
                    <EntryTable s={s} section={props.section} showVault={!props.vault} />
                </VaultSectionBox>
            </Show>
            <EntryDialogs s={s} />
        </>
    );
}
