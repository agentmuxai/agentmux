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
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
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
    const [problem, setProblem] = createSignal<string | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(false);
    const [changedMs, setChangedMs] = createSignal(0);
    return { entries, setEntries, requests, setRequests, problem, setProblem, error, setError, loading, setLoading, changedMs, setChangedMs };
});

export const vaultEntries = store.entries;
export const vaultRequests = store.requests;
/** Why the values couldn't be opened, when they couldn't. */
export const vaultProblem = store.problem;
export const vaultError = store.error;
export const vaultLoading = store.loading;
/** When the Vault last changed, for sections that load on their own. */
export const vaultChangedMs = store.changedMs;

export async function refreshVault(): Promise<void> {
    store.setLoading(true);
    try {
        const data = await call<{ entries: VaultEntryInfo[]; problem: string | null; requests: VaultRequest[] }>("list");
        store.setEntries(data.entries);
        store.setProblem(data.problem);
        store.setRequests(data.requests);
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
        const data = await call<{ requests: VaultRequest[] }>("requests");
        store.setRequests(data.requests);
    } catch (e) {
        console.log("vault: could not load agents' requests", e);
    }
}

let following = false;

/** Follow srv's `vault` event: the list is read again on any change. */
export function followVault(): void {
    if (following) return;
    following = true;
    onMuxEvent(WpsEvent.Vault, (data) => {
        store.setRequests(data.requests);
        store.setChangedMs(data.changed_ms);
        if (store.entries() !== null) void refreshVault();
    });
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

/** What the host needs to export: which entries (all when left out) and the
 *  passphrase; the host asks where to save. `null` if the person closed the dialog. */
export async function exportVault(args: { ids?: string[]; passphrase: string; defaultName: string }): Promise<{ count: number; file_name: string } | null> {
    const relay = getApi()?.approvals?.vaultExport;
    if (!relay) throw new Error("Export needs the AgentMux desktop app.");
    const res = (await relay({ ids: args.ids, passphrase: args.passphrase, default_name: args.defaultName })) as {
        cancelled?: boolean;
        count?: number;
        file_name?: string;
    };
    if (res?.cancelled) return null;
    return { count: res?.count ?? 0, file_name: res?.file_name ?? "" };
}

export type Duplicates = "skip" | "replace" | "keep_both";

/** Where an import is: the file needs its passphrase, or is read and described. */
export type ImportStep =
    | { kind: "needs_passphrase"; pickId: string; fileName: string }
    | { kind: "preview"; pickId: string; preview: ImportPreview };

/** Asks the host which file (no `pickId`) or reads the same one again with its
 *  passphrase. `null` if the person closed the file dialog. */
export async function pickImport(pickId?: string, passphrase?: string): Promise<ImportStep | null> {
    const relay = getApi()?.approvals?.vaultImport;
    if (!relay) throw new Error("Import needs the AgentMux desktop app.");
    const res = (await relay({ pick_id: pickId, passphrase })) as {
        cancelled?: boolean;
        needs_passphrase?: boolean;
        preview?: ImportPreview;
        pick_id: string;
        file_name: string;
    };
    if (res?.cancelled) return null;
    if (res.needs_passphrase) return { kind: "needs_passphrase", pickId: res.pick_id, fileName: res.file_name };
    if (!res.preview) throw new Error("AgentMux sent no preview of the file.");
    return { kind: "preview", pickId: res.pick_id, preview: res.preview };
}

export const commitImport = (token: string, duplicates: Duplicates) =>
    call<{ result: ImportResult }>("import_commit", { token, duplicates }).then((r) => r.result);
export const cancelImport = (token: string) => call<unknown>("import_cancel", { token }).catch(() => {});
export const suggestPassphrase = () => call<{ passphrase: string }>("suggest_passphrase").then((r) => r.passphrase);

export const createEntry = (entry: NewVaultEntry) => call<{ id: string }>("create", { ...entry });
export const updateEntry = (change: VaultEntryChange) => call<unknown>("update", { ...change });
export const deleteEntry = (id: string) => call<unknown>("delete", { id });
export const declineRequest = (requestId: string) => call<unknown>("decline_request", { request_id: requestId });
export const loadActivity = (limit = 200) => call<{ activity: VaultActivity[] }>("activity", { limit }).then((r) => r.activity);
export const loadKept = () => call<{ kept: KeptSecret[] }>("kept").then((r) => r.kept);
