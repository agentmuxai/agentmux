// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The groups of Settings → Notifications & tray (notifications-section.tsx lays them out): the desktop notifications
// and what they cover, the test button, the system tray rows and start at login. Each group is its rows, as a fragment.

import { Button, Select, TextInput } from "@/app/element/ui";
import { Show, type JSX } from "solid-js";

import { hostHas } from "@/util/host-caps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";
import { NOTIFICATIONS_SETTINGS } from "./notifications-settings-index";
import { s, type AutostartState } from "./notifications-section-state";

const kindRow = (entry: SettingsIndexEntry, key: string, fallback = true) => (
    <SettingRow
        id={entry.id}
        indent
        label={entry.label}
        description={entry.description}
        control={
            <ToggleControl checked={(s()[key] as boolean | undefined) ?? fallback} onChange={(v) => set(key, v)} />
        }
    />
);

/** The desktop notifications: their switch, and while it is on when to notify, which events, the quiet hours and the preview. */
export function OsNotificationsGroup(): JSX.Element {
    const osOn = () => (s()["notify:os:enabled"] as boolean | undefined) ?? true;
    return (
        <>
            <SectionHeader label="Notifications" />
            <SettingRow
                id={NOTIFICATIONS_SETTINGS.osEnabled.id}
                label={NOTIFICATIONS_SETTINGS.osEnabled.label}
                description={NOTIFICATIONS_SETTINGS.osEnabled.description}
                control={<ToggleControl checked={osOn()} onChange={(v) => set("notify:os:enabled", v)} />}
            />
            <Show when={osOn()}>
                <SettingRow
                    id={NOTIFICATIONS_SETTINGS.osWhen.id}
                    indent
                    label={NOTIFICATIONS_SETTINGS.osWhen.label}
                    description={NOTIFICATIONS_SETTINGS.osWhen.description}
                    control={
                        <Select
                            class="setting-select"
                            value={(s()["notify:os:when"] as string) ?? "unfocused"}
                            onChange={(v) => set("notify:os:when", v)}
                        >
                            <option value="unfocused">When AgentMux isn't focused</option>
                            <option value="always">Always (except the pane you're looking at)</option>
                            <option value="never">Never</option>
                        </Select>
                    }
                />
                {kindRow(NOTIFICATIONS_SETTINGS.osInputWaiting, "notify:os:inputwaiting")}
                {kindRow(NOTIFICATIONS_SETTINGS.osTurnCompleted, "notify:os:turncompleted")}
                {kindRow(NOTIFICATIONS_SETTINGS.osTurnCompletedExternal, "notify:os:turncompleted:external", false)}
                {kindRow(NOTIFICATIONS_SETTINGS.osTurnErrored, "notify:os:turnerrored")}
                {kindRow(NOTIFICATIONS_SETTINGS.osAgentCrashed, "notify:os:agentcrashed")}
                {kindRow(NOTIFICATIONS_SETTINGS.osNeedsReview, "notify:os:messageneedsreview")}
                {kindRow(NOTIFICATIONS_SETTINGS.osCloudSignedOut, "notify:os:cloudsignedout")}
                {kindRow(NOTIFICATIONS_SETTINGS.osAccountUnavailable, "notify:os:accountunavailable")}
                {kindRow(NOTIFICATIONS_SETTINGS.osJobFailed, "notify:os:jobfailed")}
                {kindRow(NOTIFICATIONS_SETTINGS.osVaultExpiring, "notify:os:vaultexpiring")}
                {kindRow(NOTIFICATIONS_SETTINGS.osSummary, "notify:os:summary")}
                {kindRow(NOTIFICATIONS_SETTINGS.taskbarAttention, "notify:taskbar:attention")}
                <SettingRow
                    id={NOTIFICATIONS_SETTINGS.quietHours.id}
                    indent
                    label={NOTIFICATIONS_SETTINGS.quietHours.label}
                    description={NOTIFICATIONS_SETTINGS.quietHours.description}
                    control={
                        <TextInput
                            class="setting-text"
                            type="text"
                            value={(s()["notify:quiethours"] as string) ?? ""}
                            placeholder="22:00-08:00"
                            onBlur={(e) => set("notify:quiethours", e.currentTarget.value.trim() || null)}
                        />
                    }
                />
                <SettingRow
                    id={NOTIFICATIONS_SETTINGS.pauseAllowAttention.id}
                    indent
                    label={NOTIFICATIONS_SETTINGS.pauseAllowAttention.label}
                    description={NOTIFICATIONS_SETTINGS.pauseAllowAttention.description}
                    control={
                        <ToggleControl
                            checked={!!(s()["notify:pause:allowattention"] as boolean)}
                            onChange={(v) => set("notify:pause:allowattention", v)}
                        />
                    }
                />
                <SettingRow
                    id={NOTIFICATIONS_SETTINGS.osPreview.id}
                    indent
                    label={NOTIFICATIONS_SETTINGS.osPreview.label}
                    description={NOTIFICATIONS_SETTINGS.osPreview.description}
                    control={
                        <Select
                            class="setting-select"
                            value={(s()["notify:os:preview"] as string) ?? "redacted"}
                            onChange={(v) => set("notify:os:preview", v)}
                        >
                            <option value="redacted">Short preview</option>
                            <option value="full">Longer preview</option>
                            <option value="none">Title only</option>
                        </Select>
                    }
                />
            </Show>
        </>
    );
}

/** The button that sends a test notification. */
export function TestNotificationRow(): JSX.Element {
    return (
        <>
            <SettingRow
                id={NOTIFICATIONS_SETTINGS.osTest.id}
                label={NOTIFICATIONS_SETTINGS.osTest.label}
                description={NOTIFICATIONS_SETTINGS.osTest.description}
                control={
                    <Button
                        onClick={() => void RpcApi.NotifyTestCommand(TabRpcClient).catch(() => {})}
                    >
                        Send
                    </Button>
                }
            />
        </>
    );
}

/** The system tray rows, on a host that has a tray. */
export function TrayGroup(): JSX.Element {
    return (
        <>
            {/* Desktop-only: a host without a tray or login entries doesn't show these. */}
            <Show when={hostHas("tray") || hostHas("autostart")}>
                <SectionHeader label="System tray" />
            </Show>
            <Show when={hostHas("tray")}>
                <SettingRow
                    id={NOTIFICATIONS_SETTINGS.showTray.id}
                    label={NOTIFICATIONS_SETTINGS.showTray.label}
                    description={NOTIFICATIONS_SETTINGS.showTray.description}
                    control={
                        <ToggleControl
                            checked={(s()["app:showtray"] as boolean | undefined) ?? true}
                            onChange={(v) => set("app:showtray", v)}
                        />
                    }
                />
                <SettingRow
                    id={NOTIFICATIONS_SETTINGS.runInBackground.id}
                    label={NOTIFICATIONS_SETTINGS.runInBackground.label}
                    description={NOTIFICATIONS_SETTINGS.runInBackground.description}
                    control={
                        <ToggleControl
                            checked={(s()["app:runinbackground"] as boolean | undefined) ?? false}
                            onChange={(v) => set("app:runinbackground", v)}
                        />
                    }
                />
            </Show>
        </>
    );
}

/** Start at login, on a host that manages login entries. */
export function StartAtLoginRow({ a }: { a: AutostartState }): JSX.Element {
    return (
        <>
            <Show when={hostHas("autostart")}>
                <SettingRow
                    id={NOTIFICATIONS_SETTINGS.autostart.id}
                    label={NOTIFICATIONS_SETTINGS.autostart.label}
                    description={a.autostartDescription()}
                    control={
                        <Show
                            when={a.autostart()?.available !== false}
                            fallback={<ToggleControl checked={false} onChange={() => {}} />}
                        >
                            <ToggleControl checked={a.startAtLogin()} onChange={(v) => set("app:startatlogin", v)} />
                        </Show>
                    }
                />
            </Show>
        </>
    );
}
