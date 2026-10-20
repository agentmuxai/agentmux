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

export const createEntry = (entry: NewVaultEntry) => call<{ id: string }>("create", { ...entry });
export const updateEntry = (change: VaultEntryChange) => call<unknown>("update", { ...change });
export const deleteEntry = (id: string) => call<unknown>("delete", { id });
export const declineRequest = (requestId: string) => call<unknown>("decline_request", { request_id: requestId });
export const loadActivity = (limit = 200) => call<{ activity: VaultActivity[] }>("activity", { limit }).then((r) => r.activity);
export const loadKept = () => call<{ kept: KeptSecret[] }>("kept").then((r) => r.kept);
