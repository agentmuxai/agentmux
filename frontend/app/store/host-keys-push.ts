// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Browser panes follow the person's keybindings (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §7).
// A focused browser pane is a native window whose keys the app never sees; the host forwards the ones in its list,
// which starts as the default table's. This sends it the list the person's own table gives whenever the keybindings
// change, so a key they bind is forwarded (and not run by the page) and a key they unbind goes back to the page.

import { createEffect, createRoot, on } from "solid-js";

import { keyPlatform } from "@/app/keybindings";
import { hostKeys, hostKeysOf } from "@/app/keybindings/host-keys";
import { effectiveRows, keybindingsVersion } from "@/app/keybindings/registry";
import { getApi } from "@/app/store/global";

/** What the host has: the generated list it starts with, until the first push. */
let sent: string | undefined;

/** Sends the host the keys for the table as it is now, when they differ from what it has. */
export function pushHostKeys(): void {
    const platform = keyPlatform();
    const keys = hostKeysOf(effectiveRows(platform), platform);
    const json = JSON.stringify(keys);
    if (json === (sent ?? JSON.stringify(hostKeys(platform)))) return;
    sent = json;
    getApi().setHostKeys?.(keys);
}

/** For a test: the host is as it started. */
export function resetHostKeysPush(): void {
    sent = undefined;
}

/** Pushes now and whenever the keybindings change. */
export function registerHostKeysPush(): void {
    createRoot(() => createEffect(on(keybindingsVersion, pushHostKeys)));
}
