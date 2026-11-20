// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Notifications & tray — docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md.
// Phase 0 (§4.1): make the existing tray + background-service mode reachable
// from Settings instead of env vars only, and surface auto-start as its own,
// separate toggle (tray spec §7.4 — the two decisions stay independent).


import { createMemo, createResource, createSignal } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { getApi } from "@/util/app-api";
import { settingsAtom } from "@/app/store/global";
import { set } from "../settings-controls";
import { NOTIFICATIONS_SETTINGS } from "./notifications-settings-index";

export const s = () => settingsAtom() ?? ({} as any);

/** Time for the launcher to apply a change before the registration is re-read. */
const AUTOSTART_SETTLE_MS = 2000;

/** Between switching start at login off and on again for a repair: long enough for the launcher to see both values. */
const REPAIR_GAP_MS = 400;

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
    // Bumped after a repair, which can end on the value the setting had, so the read-back still runs.
    const [repairs, setRepairs] = createSignal(0);
    const [autostart] = createResource(
        () => ({ on: startAtLogin(), repairs: repairs() }),
        async () => {
            if (autostartFetched) await new Promise((r) => setTimeout(r, AUTOSTART_SETTLE_MS));
            autostartFetched = true;
            return fetchAutostart();
        },
    );
    // What the one button next to the switch does, when there is something to do: a login entry whose file is gone
    // is rewritten ("repair"); another build's entry is taken over ("use this build"). The setting is written off
    // and on again, which the launcher reconciles like any change (`autostart::plan` writes for a missing target
    // and for another channel's entry), so no second path to the login entry exists.
    const fix = (): "repair" | "use-this-build" | null => {
        const st = autostart();
        if (autostart.loading || !st || st.available === false) return null;
        if (st.state === "broken") return "repair";
        if (st.state === "on" && st.movable) return "use-this-build";
        return null;
    };
    const applyFix = () => {
        set("app:startatlogin", false);
        setTimeout(() => {
            set("app:startatlogin", true);
            setRepairs((n) => n + 1);
        }, REPAIR_GAP_MS);
    };
    const autostartDescription = () => {
        const st = autostart();
        if (st?.available === false) return "Unavailable in this build (not started by the AgentMux launcher).";
        if (fix() === "repair") return t("settings.notifications.autostart.broken");
        if (fix() === "use-this-build") {
            return st?.ownerChannel
                ? t("settings.notifications.autostart.owner", { channel: st.ownerChannel })
                : t("settings.notifications.autostart.ownerUnknown");
        }
        if (startAtLogin() && st && !autostart.loading && !st.enabled) {
            return "On, but not registered with the system yet. If this persists, see the launcher log.";
        }
        return NOTIFICATIONS_SETTINGS.autostart.description;
    };

    return { startAtLogin, autostart, autostartDescription, fix, applyFix };
}

export type AutostartState = ReturnType<typeof createAutostart>;
