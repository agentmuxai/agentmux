// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The named browser profiles, as one reactive list shared by every browser
 * pane's menu, its tab marks and the Settings section
 * (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3–§4). Loaded
 * on first use and after every change made here; a change made in another
 * window shows the next time a menu opens (it reloads).
 */

import { createSignal } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { farthestFreeSlot } from "@/app/theme/palette";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BrowserProfile } from "@/types/rpc/BrowserProfile";
import type { BrowserProfilesResult } from "@/types/rpc/BrowserProfilesResult";

const [profiles, setProfilesOnly] = createSignal<BrowserProfile[]>([]);
const [signins, setSignins] = createSignal<Record<string, number>>({});
let loaded = false;

/** Takes srv's answer: the profiles, and how many saved sign-ins each holds. */
function setProfiles(r: BrowserProfilesResult): void {
    setProfilesOnly(r.profiles ?? []);
    setSignins(Object.fromEntries(Object.entries(r.signins ?? {}).map(([id, n]) => [id, n ?? 0])));
}

/** How many sign-ins the Vault keeps for profile `id`. */
export function profileSignins(id: string): number {
    return signins()[id] ?? 0;
}

/** The profiles there are (Personal isn't listed: it always exists). */
export const browserProfiles = profiles;

/** The colour srv would give a profile made now (`palette.rs` farthest_free_slot),
 *  so most people never choose. */
export function nextProfileColor(): string {
    return farthestFreeSlot(profiles().map((p) => p.color)).hex;
}

export async function loadBrowserProfiles(): Promise<void> {
    loaded = true;
    try {
        setProfiles(await RpcApi.ListBrowserProfilesCommand(TabRpcClient));
    } catch {
        // Keep the last list: a menu still works with it.
    }
}

/** Load once, on first use. */
export function ensureBrowserProfiles(): void {
    if (!loaded) void loadBrowserProfiles();
}

export function profileById(id: string): BrowserProfile | undefined {
    return profiles().find((p) => p.id === id);
}

export async function createBrowserProfile(name: string, color?: string): Promise<BrowserProfile> {
    const r = await RpcApi.CreateBrowserProfileCommand(TabRpcClient, { name, color });
    setProfiles(r);
    if (!r.created) throw new Error("the profile wasn't created");
    return r.created;
}

export async function updateBrowserProfile(
    id: string,
    patch: { name?: string; color?: string; agents_allowed?: boolean }
): Promise<void> {
    setProfiles(await RpcApi.UpdateBrowserProfileCommand(TabRpcClient, { id, ...patch }));
}

/**
 * Deletes profile `id`: its open tabs close, and its saved sign-ins move to
 * Personal, or go with it when `deleteSignins`.
 */
export async function deleteBrowserProfile(id: string, deleteSignins = false): Promise<void> {
    setProfiles(await RpcApi.DeleteBrowserProfileCommand(TabRpcClient, { id, delete_signins: deleteSignins }));
}

/** A profile's initial, for its badge. */
export function profileInitial(name: string): string {
    return (name.trim()[0] ?? "?").toUpperCase();
}
