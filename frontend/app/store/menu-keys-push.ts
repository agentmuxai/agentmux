// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The macOS menu bar shows the keys of the person's keybindings. It is native, so the host can't read the table;
// this sends it the key of each command now, and again whenever the keybindings change (menu-keys.ts says which).

import { createEffect, createRoot, on } from "solid-js";

import { keyPlatform } from "@/app/keybindings";
import { menuKeysOf } from "@/app/keybindings/menu-keys";
import { effectiveRows, keybindingsVersion } from "@/app/keybindings/registry";
import { getApi } from "@/app/store/global";

/** What the host has: nothing, until the first push. */
let sent: string | undefined;

/** Sends the host the menu keys for the table as it is now, when they differ from what it has. macOS only. */
export function pushMenuKeys(): void {
    const platform = keyPlatform();
    if (platform !== "mac") return;
    const keys = menuKeysOf(effectiveRows(platform), platform);
    const json = JSON.stringify(keys);
    if (json === sent) return;
    sent = json;
    getApi().setMenuKeys?.(keys);
}

/** For a test: the host has nothing. */
export function resetMenuKeysPush(): void {
    sent = undefined;
}

/** Pushes now and whenever the keybindings change. */
export function registerMenuKeysPush(): void {
    createRoot(() => createEffect(on(keybindingsVersion, pushMenuKeys)));
}
