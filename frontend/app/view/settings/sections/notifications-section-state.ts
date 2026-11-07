// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Notifications & tray — docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md.
// Phase 0 (§4.1): make the existing tray + background-service mode reachable
// from Settings instead of env vars only, and surface auto-start as its own,
// separate toggle (tray spec §7.4 — the two decisions stay independent).


import { createMemo, createResource } from "solid-js";

import { getApi } from "@/util/app-api";
import { settingsAtom } from "@/app/store/global";
import { NOTIFICATIONS_SETTINGS } from "./notifications-settings-index";

export const s = () => settingsAtom() ?? ({} as any);

/** Time for the launcher to apply a change before the registration is re-read. */
const AUTOSTART_SETTLE_MS = 2000;

async function fetchAutostart(): Promise<AutostartStatus> {
    try {
        return await getApi().getAutostartStatus();
    } catch {
        // Older host without the verb, or no launcher (standalone dev host).
        return { available: false, enabled: false };
    }
}

/** The start-at-login switch and the registration read back from the launcher. */
export function createAutostart() {
    // One property, two switches: this toggle and the tray menu's check item
    // both write `app:startatlogin`, and the launcher applies it to the OS login
    // entry (SPEC_START_WITH_OS_2026_09_25.md §3.9). Off by default.
    // A memo, because `settingsAtom` is replaced on every settings broadcast:
    // it notifies only when this key's value actually changes (ReAgent P1 on
    // #3788), so an unrelated setting never re-reads the registration.
    const startAtLogin = createMemo(() => (s()["app:startatlogin"] as boolean | undefined) ?? false);
    // The registration is read back after every change (and once on open), only
    // to report availability and a failed registration. The source is wrapped
    // in an object because a `false` source would stop the resource fetching.
    let autostartFetched = false;
    const [autostart] = createResource(
        () => ({ on: startAtLogin() }),
        async () => {
            if (autostartFetched) await new Promise((r) => setTimeout(r, AUTOSTART_SETTLE_MS));
            autostartFetched = true;
            return fetchAutostart();
        },
    );
    const autostartDescription = () => {
        const st = autostart();
        if (st?.available === false) return "Unavailable in this build (not started by the AgentMux launcher).";
        if (startAtLogin() && st && !autostart.loading && !st.enabled) {
            return "On, but not registered with the system yet. If this persists, see the launcher log.";
        }
        return NOTIFICATIONS_SETTINGS.autostart.description;
    };

    return { startAtLogin, autostart, autostartDescription };
}

export type AutostartState = ReturnType<typeof createAutostart>;
