// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault pane's data. Every operation goes through the host
// (`approvals.vault`), which srv trusts and agents can't impersonate
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.4).
// srv never sends a value back: the pane sees labels, plain fields, which
// parts are set, and the main value's last characters.

import { createRoot, createSignal } from "solid-js";

import { getApi } from "@/app/store/app-api";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import type { ImportPreview } from "@/types/rpc/ImportPreview";
import type { ImportResult } from "@/types/rpc/ImportResult";
import type { MappingNeeded } from "@/types/rpc/MappingNeeded";
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultVersionInfo } from "@/types/rpc/VaultVersionInfo";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";

export interface KeptSecret {
    label: string;
    what: string;
    /** Where it is managed: `accounts`, `muxbus` or `tower`. */
    managed_in: string;
}

export interface NewVaultEntry {
    kind: string;
    label: string;
    fields: Record<string, string>;
    tags?: string[];
    parts: Record<string, string>;
    /** The agent request this entry fills, if any. */
    request_id?: string;
}

export interface VaultEntryChange {
    id: string;
    label: string;
    fields: Record<string, string>;
    tags?: string[];
    /** Only the parts to replace; a part left out keeps its value. */
    parts: Record<string, string>;
    clear?: string[];
}

export function vaultAvailable(): boolean {
    return typeof getApi()?.approvals?.vault === "function";
}

async function call<T>(op: string, args: Record<string, unknown> = {}): Promise<T> {
    const relay = getApi()?.approvals?.vault;
    if (!relay) throw new Error("The Vault needs the AgentMux desktop app.");
    return (await relay(op, args)) as T;
}

const store = createRoot(() => {
    const [entries, setEntries] = createSignal<VaultEntryInfo[] | null>(null);
    const [requests, setRequests] = createSignal<VaultRequest[]>([]);
    const [uses, setUses] = createSignal<UseRequest[]>([]);
    const [problem, setProblem] = createSignal<string | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(false);
    const [changedMs, setChangedMs] = createSignal(0);
    return { entries, setEntries, requests, setRequests, uses, setUses, problem, setProblem, error, setError, loading, setLoading, changedMs, setChangedMs };
});

export const vaultEntries = store.entries;
export const vaultRequests = store.requests;
/** Agents' uses of an entry (filling a web form) waiting for the person to allow them. */
export const vaultUses = store.uses;
/** Why the values couldn't be opened, when they couldn't. */
export const vaultProblem = store.problem;
export const vaultError = store.error;
export const vaultLoading = store.loading;
/** When the Vault last changed, for sections that load on their own. */
export const vaultChangedMs = store.changedMs;

export async function refreshVault(): Promise<void> {
    store.setLoading(true);
    try {
        const data = await call<{ entries: VaultEntryInfo[]; problem: string | null; requests: VaultRequest[]; uses: UseRequest[] }>("list");
        store.setEntries(data.entries);
        store.setProblem(data.problem);
        store.setRequests(data.requests);
        store.setUses(data.uses ?? []);
        store.setError(null);
    } catch (e) {
        store.setError(e instanceof Error ? e.message : String(e));
    } finally {
        store.setLoading(false);
    }
}

/** Only the agents' requests: needs no keychain read. */
export async function refreshVaultRequests(): Promise<void> {
    try {
        const data = await call<{ requests: VaultRequest[]; uses: UseRequest[] }>("requests");
        store.setRequests(data.requests);
        store.setUses(data.uses ?? []);
    } catch (e) {
        console.log("vault: could not load agents' requests", e);
    }
}

let following = false;
/** How many Vault views are on screen. The list opens the Vault key and decrypts every entry, which can
 *  make the OS ask the person for it, so it is read again only while one is looking. */
let holders = 0;

/** Follow srv's `vault` event: the agents' requests and uses at once, and the list again on any change
 *  while a Vault view is on screen (what changed meanwhile is read when one next is). */
export function followVault(): void {
    if (following) return;
    following = true;
    onMuxEvent(WpsEvent.Vault, (data) => {
        store.setRequests(data.requests);
        store.setUses(data.uses);
        store.setChangedMs(data.changed_ms);
        if (holders > 0) void refreshVault();
    });
}

/** A Vault view is on screen until the returned function is called: the list is read when the first one
 *  appears, and kept current while any is. Opening the Vault's key happens only through this. */
export function holdVaultShown(): () => void {
    if (holders++ === 0) void refreshVault();
    let released = false;
    return () => {
        if (released) return;
        released = true;
        holders--;
    };
}

/** Tells srv the Vault pane `pane` is showing, or no longer is: agents'
 *  whole-window captures of AgentMux are refused while one does (spec §5.3).
 *  Each pane has its own note, so one pane hiding can't clear another's. */
export const setVaultOnScreen = (pane: string, on: boolean) => call<unknown>("on_screen", { pane, on });

/** One part's value, after the OS (Windows Hello) confirms the person: the host
 *  asks, then fetches it from srv. Rejects with "cancelled" if they dismiss it. */
export async function revealVaultPart(id: string, part: string, label: string): Promise<string> {
    const relay = getApi()?.approvals?.vaultReveal;
    if (!relay) throw new Error("Reveal needs the AgentMux desktop app.");
    const res = (await relay({ id, part, label })) as { value?: string };
    if (typeof res?.value !== "string") throw new Error("AgentMux sent no value.");
    return res.value;
}

/** Copies one part's value on the host, which clears it later; the value never
 *  reaches this page. Answers how many seconds until it clears. */
export async function copyVaultPart(id: string, part: string, label: string): Promise<number> {
    const relay = getApi()?.approvals?.vaultCopy;
    if (!relay) throw new Error("Copy needs the AgentMux desktop app.");
    const res = (await relay({ id, part, label })) as { clears_in_secs?: number };
    return res?.clears_in_secs ?? 30;
}

export type UseDecision = "once" | "always" | "deny";

/** The person's answer to an agent's use of an entry; srv takes it only from the host. */
export const answerUse = (id: string, decision: UseDecision, typed?: { value: string; save: boolean }) =>
    call<unknown>("answer_use", { id, decision, ...(typed ? { value: typed.value, save: typed.save } : {}) });
/** Stops an agent filling an entry on a site without asking. */
export const revokeGrant = (entryId: string, origin: string) => call<unknown>("revoke_grant", { entry_id: entryId, origin });

/** What the host needs to export: which entries (all when left out) and the
 *  passphrase; the host asks where to save. `null` if the person closed the dialog. */
export async function exportVault(args: { ids?: string[]; passphrase: string; defaultName: string; format?: "vault" | "kdbx" }): Promise<{ count: number; file_name: string } | null> {
    const relay = getApi()?.approvals?.vaultExport;
    if (!relay) throw new Error("Export needs the AgentMux desktop app.");
    const res = (await relay({ ids: args.ids, passphrase: args.passphrase, default_name: args.defaultName, format: args.format })) as {
        cancelled?: boolean;
        count?: number;
        file_name?: string;
    };
    if (res?.cancelled) return null;
    return { count: res?.count ?? 0, file_name: res?.file_name ?? "" };
}

/** The two unencrypted formats, for taking the Vault to another manager. */
export type PlainFormat = "csv" | "json";

/** Where an unencrypted export will be written, before anything is. */
export interface PlainPlace {
    pickId: string;
    fileName: string;
    folder: string;
    /** The cloud service that syncs that folder, if one does. */
    syncedFolder: string | null;
}

/** Step one of an unencrypted export: the host asks where to save. `null` if the person closed the dialog. */
export async function choosePlainPlace(args: { format: PlainFormat; defaultName: string }): Promise<PlainPlace | null> {
    const relay = getApi()?.approvals?.vaultExport;
    if (!relay) throw new Error("Export needs the AgentMux desktop app.");
    const res = (await relay({ format: args.format, default_name: args.defaultName })) as { cancelled?: boolean; pick_id?: string; file_name?: string; folder?: string; synced_folder?: string | null };
    if (res?.cancelled || !res?.pick_id) return null;
    return { pickId: res.pick_id, fileName: res.file_name ?? "", folder: res.folder ?? "", syncedFolder: res.synced_folder ?? null };
}

/** Step two: after the person typed the confirmation, the host has the OS confirm them and srv writes the file. */
export async function exportPlain(args: { format: PlainFormat; pickId: string; ids?: string[] }): Promise<{ count: number; file_name: string }> {
    const relay = getApi()?.approvals?.vaultExport;
    if (!relay) throw new Error("Export needs the AgentMux desktop app.");
    const res = (await relay({ format: args.format, pick_id: args.pickId, ids: args.ids, confirmed: true })) as { count?: number; file_name?: string };
    return { count: res?.count ?? 0, file_name: res?.file_name ?? "" };
}

/** The person backed out after choosing the place: the host forgets it. */
export async function cancelPlainPlace(format: PlainFormat): Promise<void> {
    await getApi()?.approvals?.vaultExport?.({ format, cancel: true }).catch(() => undefined);
}

export type Duplicates = "skip" | "replace" | "keep_both";

/** Where an import is: the file needs its passphrase, or is read and described. */
export type ImportStep =
    | { kind: "needs_passphrase"; pickId: string; fileName: string; keyFileName: string | null }
    /** A CSV no reader knows: the person says which column is which. */
    | { kind: "needs_mapping"; pickId: string; fileName: string; columns: string[]; rows: number }
    | { kind: "preview"; pickId: string; preview: ImportPreview; syncedFolder: string | null };

/** The person's column choices for a CSV no reader knows. */
export interface CsvMapping {
    kind: string;
    label: number;
    value: number;
    site?: number;
    username?: number;
}

/** Asks the host which file (no `pickId`) or reads the same one again with its
 *  passphrase. `null` if the person closed the file dialog. */
export async function pickImport(pickId?: string, passphrase?: string, mapping?: CsvMapping, keyFile?: "choose" | "clear"): Promise<ImportStep | null> {
    const relay = getApi()?.approvals?.vaultImport;
    if (!relay) throw new Error("Import needs the AgentMux desktop app.");
    const res = (await relay({ pick_id: pickId, passphrase, mapping, choose_key_file: keyFile === "choose" || undefined, clear_key_file: keyFile === "clear" || undefined })) as {
        cancelled?: boolean;
        /** The key file the person chose for a KeePass database, by name only. */
        key_file_name?: string | null;
        needs_passphrase?: boolean;
        needs_mapping?: MappingNeeded;
        preview?: ImportPreview;
        synced_folder?: string | null;
        pick_id: string;
        file_name: string;
    };
    if (res?.cancelled) return null;
    if (res.needs_passphrase) return { kind: "needs_passphrase", pickId: res.pick_id, fileName: res.file_name, keyFileName: res.key_file_name ?? null };
    if (res.needs_mapping) return { kind: "needs_mapping", pickId: res.pick_id, fileName: res.file_name, columns: res.needs_mapping.columns, rows: res.needs_mapping.rows };
    if (!res.preview) throw new Error("AgentMux sent no preview of the file.");
    return { kind: "preview", pickId: res.pick_id, preview: res.preview, syncedFolder: res.synced_folder ?? null };
}

/** Deletes the file the person picked, by its id (the host holds the path). Not a
 *  secure erase: on an SSD an overwrite doesn't reliably reach the old blocks. */
export async function deleteImportFile(pickId: string): Promise<void> {
    const relay = getApi()?.approvals?.vaultImportDelete;
    if (!relay) throw new Error("Deleting needs the AgentMux desktop app.");
    await relay({ pick_id: pickId });
}

export const commitImport = (token: string, duplicates: Duplicates) =>
    call<{ result: ImportResult }>("import_commit", { token, duplicates }).then((r) => r.result);
export const cancelImport = (token: string) => call<unknown>("import_cancel", { token }).catch(() => {});
export const suggestPassphrase = () => call<{ passphrase: string }>("suggest_passphrase").then((r) => r.passphrase);

export const createEntry = (entry: NewVaultEntry) => call<{ id: string }>("create", { ...entry });
export const updateEntry = (change: VaultEntryChange) => call<unknown>("update", { ...change });
export const deleteEntry = (id: string) => call<unknown>("delete", { id });
/** An entry's earlier values, kept 7 days: when and by whom each was replaced, and its last characters. */
export const loadVersions = (id: string) => call<{ versions: VaultVersionInfo[] }>("versions", { id }).then((r) => r.versions);
/** Puts an earlier value back; the current one is kept in its place. */
export const restoreVersion = (id: string, seq: number) => call<unknown>("restore", { id, seq });
export const declineRequest = (requestId: string) => call<unknown>("decline_request", { request_id: requestId });
export const loadActivity = (limit = 200) => call<{ activity: VaultActivity[] }>("activity", { limit }).then((r) => r.activity);
/** An entry with standing permissions, from a read that needs no key (Warden's view). */
export interface GrantedEntry {
    id: string;
    label: string;
    kind: string;
    grants: string[];
}
export const loadGrants = () => call<{ granted: GrantedEntry[] }>("grants").then((r) => r.granted);
export const loadKept = () => call<{ kept: KeptSecret[] }>("kept").then((r) => r.kept);
