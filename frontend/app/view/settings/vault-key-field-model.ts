// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A settings key kept in the Vault: how a setting names its entry (`vault:<entry id>`, the spelling srv reads in
// crates/srv/src/backend/vault/setting_key.rs) and which entries can hold such a key.

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

const PREFIX = "vault:";

/** The entry id a setting value names, when it is a `vault:<id>` reference. */
export function entryIdOf(value: string | undefined | null): string | undefined {
    const id = value?.trim().startsWith(PREFIX) ? value.trim().slice(PREFIX.length).trim() : "";
    return id || undefined;
}

/** The setting value that names entry `id`. */
export function referenceTo(id: string): string {
    return `${PREFIX}${id}`;
}

/** The entries a service key can be kept in: API keys, by label. */
export function keyChoices(entries: readonly VaultEntryInfo[]): VaultEntryInfo[] {
    return entries.filter((e) => e.kind === "api-key").sort((a, b) => a.label.localeCompare(b.label));
}

/** What the key setting holds, for what the control says. `plain` is a key still in settings.json (the
 *  renderer is given a placeholder for it, never the key). */
export type KeyState = "unset" | "plain" | "vault" | "gone";

export function keyState(value: string | undefined | null, entries: readonly VaultEntryInfo[] | null): KeyState {
    if (!value?.trim()) return "unset";
    const id = entryIdOf(value);
    if (!id) return "plain";
    // Until the Vault has loaded, a reference is taken as in place rather than flashed as gone.
    if (entries === null) return "vault";
    return keyChoices(entries).some((e) => e.id === id) ? "vault" : "gone";
}
