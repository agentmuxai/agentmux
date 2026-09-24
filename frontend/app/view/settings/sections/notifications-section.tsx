// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Notifications & tray — docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md.
// Phase 0 (§4.1): make the existing tray + background-service mode reachable
// from Settings instead of env vars only, and surface auto-start as its own,
// separate toggle (tray spec §7.4 — the two decisions stay independent).

import { createResource, createSignal, Show, type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const NOTIFICATIONS_SETTINGS = {
    runInBackground: {
        id: "notifications.run_in_background",
        label: "Keep running in the system tray",
        description:
            "When all windows are closed, AgentMux keeps running with an icon in the system tray instead of quitting. Applies at the next launch. On Windows 11 new tray icons start in the overflow (^) area — drag it onto the taskbar to keep it visible.",
        section: "notifications",
        keywords: ["system tray", "tray icon", "menu bar", "background", "minimize to tray", "close to tray", "app:runinbackground"],
    },
    autostart: {
        id: "notifications.autostart",
        label: "Start at login",
        description: "Start AgentMux in the background (tray only, no window) when you log in.",
        section: "notifications",
        keywords: ["auto start", "autostart", "launch at login", "startup", "run at boot", "login items"],
    },
} satisfies Record<string, SettingsIndexEntry>;

type AutostartStatus = { available: boolean; enabled: boolean };

async function fetchAutostart(): Promise<AutostartStatus> {
    try {
        const { invokeCommand } = await import("@/app/platform/ipc");
        return ((await invokeCommand("autostart_status", {})) as AutostartStatus) ?? { available: false, enabled: false };
    } catch {
        // Older host without the verb, or no launcher (standalone dev host).
        return { available: false, enabled: false };
    }
}

// ── Section: Notifications & Tray ─────────────────────────────────────────────

export function NotificationsSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const [autostart, { mutate }] = createResource(fetchAutostart);
    const [autostartError, setAutostartError] = createSignal<string | null>(null);

    const toggleAutostart = async (enabled: boolean) => {
        setAutostartError(null);
        try {
            const { invokeCommand } = await import("@/app/platform/ipc");
            mutate((await invokeCommand("set_autostart", { enabled })) as AutostartStatus);
        } catch (e) {
            setAutostartError(String(e));
        }
    };

    return (
        <div class="settings-section-body">
            <SectionHeader label="System tray" />
            <SettingRow
                id={NOTIFICATIONS_SETTINGS.runInBackground.id}
                label={NOTIFICATIONS_SETTINGS.runInBackground.label}
                description={NOTIFICATIONS_SETTINGS.runInBackground.description}
                control={
                    <ToggleControl
                        checked={!!(s()["app:runinbackground"] as boolean)}
                        onChange={(v) => set("app:runinbackground", v)}
                    />
                }
            />
            <SettingRow
                id={NOTIFICATIONS_SETTINGS.autostart.id}
                label={NOTIFICATIONS_SETTINGS.autostart.label}
                description={
                    autostart()?.available === false
                        ? "Unavailable in this build (not started by the AgentMux launcher)."
                        : (autostartError() ?? NOTIFICATIONS_SETTINGS.autostart.description)
                }
                control={
                    <Show when={autostart()?.available} fallback={<ToggleControl checked={false} onChange={() => {}} />}>
                        <ToggleControl checked={!!autostart()?.enabled} onChange={(v) => void toggleAutostart(v)} />
                    </Show>
                }
            />
        </div>
    );
}
