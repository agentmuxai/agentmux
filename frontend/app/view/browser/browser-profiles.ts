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
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BrowserProfile } from "@/types/rpc/BrowserProfile";

const [profiles, setProfiles] = createSignal<BrowserProfile[]>([]);
let loaded = false;

/** The profiles there are (Personal isn't listed: it always exists). */
export const browserProfiles = profiles;

/** Colours the New-profile dialog offers (the same palette srv picks from). */
export const PROFILE_COLORS = ["#3b82f6", "#22c55e", "#f59e0b", "#ef4444", "#a855f7", "#14b8a6", "#ec4899", "#64748b"];

export async function loadBrowserProfiles(): Promise<void> {
    loaded = true;
    try {
        const r = await RpcApi.ListBrowserProfilesCommand(TabRpcClient);
        setProfiles(r.profiles ?? []);
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
    setProfiles(r.profiles ?? []);
    if (!r.created) throw new Error("the profile wasn't created");
    return r.created;
}

export async function updateBrowserProfile(id: string, patch: { name?: string; color?: string }): Promise<void> {
    const r = await RpcApi.UpdateBrowserProfileCommand(TabRpcClient, { id, ...patch });
    setProfiles(r.profiles ?? []);
}

/** Deletes profile `id`: its open tabs close and its saved sign-ins go. */
export async function deleteBrowserProfile(id: string): Promise<void> {
    const r = await RpcApi.DeleteBrowserProfileCommand(TabRpcClient, { id });
    setProfiles(r.profiles ?? []);
}

/** A profile's initial, for its badge. */
export function profileInitial(name: string): string {
    return (name.trim()[0] ?? "?").toUpperCase();
}
