// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Notifications & tray — docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md.
// Phase 0 (§4.1): make the existing tray + background-service mode reachable
// from Settings instead of env vars only, and surface auto-start as its own,
// separate toggle (tray spec §7.4 — the two decisions stay independent).

import { createResource, createSignal, Show, type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
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
    osEnabled: {
        id: "notifications.os_enabled",
        label: "Desktop notifications",
        description: "Show a system notification when an agent needs you or finishes while you're not looking at it.",
        section: "notifications",
        keywords: ["toast", "os notification", "system notification", "alert", "action center", "notify:os:enabled"],
    },
    osWhen: {
        id: "notifications.os_when",
        label: "When to notify",
        description: "\u201cWhen AgentMux isn't focused\u201d still alerts you if an agent needs input in a pane you aren't looking at.",
        section: "notifications",
        keywords: ["focus", "background only", "always notify", "notify:os:when"],
    },
    osInputWaiting: {
        id: "notifications.os_input_waiting",
        label: "Agent needs input",
        description: "An agent asked a question and is waiting for your answer",
        section: "notifications",
        keywords: ["question", "waiting", "blocked", "notify:os:inputwaiting"],
    },
    osTurnCompleted: {
        id: "notifications.os_turn_completed",
        label: "Agent finished",
        description: "An agent's turn completed",
        section: "notifications",
        keywords: ["done", "finished", "complete", "notify:os:turncompleted"],
    },
    osTurnErrored: {
        id: "notifications.os_turn_errored",
        label: "Agent stopped with an error",
        description: "An agent's turn ended with an error",
        section: "notifications",
        keywords: ["error", "failure", "crash", "notify:os:turnerrored"],
    },
    osAgentCrashed: {
        id: "notifications.os_agent_crashed",
        label: "Agent stopped unexpectedly",
        description: "Sign-in expired, usage limit, crash — not when you stop it yourself",
        section: "notifications",
        keywords: ["crash", "auth expired", "usage limit", "notify:os:agentcrashed"],
    },
    osNeedsReview: {
        id: "notifications.os_needs_review",
        label: "Message needs your review",
        description: "An agent received a message it must not act on without you. The notification never shows the message.",
        section: "notifications",
        keywords: ["jekt", "sensitive", "escalate", "review", "notify:os:messageneedsreview"],
    },
    pauseAllowAttention: {
        id: "notifications.pause_allow_attention",
        label: "While paused, still alert when an agent needs me",
        description: "Pause from the tray icon's menu. Input requests and review requests still get through.",
        section: "notifications",
        keywords: ["pause", "snooze", "do not disturb", "notify:pause:allowattention"],
    },
    taskbarAttention: {
        id: "notifications.taskbar_attention",
        label: "Badge the taskbar button",
        description: "Show a dot on AgentMux's taskbar button, and flash it once, while an agent needs you (Windows)",
        section: "notifications",
        keywords: ["taskbar", "badge", "flash", "overlay", "notify:taskbar:attention"],
    },
    quietHours: {
        id: "notifications.quiet_hours",
        label: "Quiet hours",
        description: "Hold notifications back every day during these hours (local time), e.g. 22:00-08:00. Leave empty for none.",
        section: "notifications",
        keywords: ["do not disturb", "night", "schedule", "sleep", "notify:quiethours"],
    },
    osPreview: {
        id: "notifications.os_preview",
        label: "Notification content",
        description: "How much of an agent's question to show. Credentials and sensitive terms are always hidden.",
        section: "notifications",
        keywords: ["privacy", "preview", "lock screen", "notify:os:preview"],
    },
    osTest: {
        id: "notifications.os_test",
        label: "Send test notification",
        description: "Check that notifications reach your desktop (Windows Focus / Do Not Disturb may hold it in the notification center).",
        section: "notifications",
        keywords: ["test toast", "try notification"],
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

    const osOn = () => (s()["notify:os:enabled"] as boolean | undefined) ?? true;
    const kindRow = (entry: SettingsIndexEntry, key: string) => (
        <SettingRow
            id={entry.id}
            indent
            label={entry.label}
            description={entry.description}
            control={
                <ToggleControl checked={(s()[key] as boolean | undefined) ?? true} onChange={(v) => set(key, v)} />
            }
        />
    );

    return (
        <div class="settings-section-body">
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
                        <select
                            class="setting-select"
                            value={(s()["notify:os:when"] as string) ?? "unfocused"}
                            onChange={(e) => set("notify:os:when", e.currentTarget.value)}
                        >
                            <option value="unfocused">When AgentMux isn't focused</option>
                            <option value="always">Always (except the pane you're looking at)</option>
                            <option value="never">Never</option>
                        </select>
                    }
                />
                {kindRow(NOTIFICATIONS_SETTINGS.osInputWaiting, "notify:os:inputwaiting")}
                {kindRow(NOTIFICATIONS_SETTINGS.osTurnCompleted, "notify:os:turncompleted")}
                {kindRow(NOTIFICATIONS_SETTINGS.osTurnErrored, "notify:os:turnerrored")}
                {kindRow(NOTIFICATIONS_SETTINGS.osAgentCrashed, "notify:os:agentcrashed")}
                {kindRow(NOTIFICATIONS_SETTINGS.osNeedsReview, "notify:os:messageneedsreview")}
                {kindRow(NOTIFICATIONS_SETTINGS.taskbarAttention, "notify:taskbar:attention")}
                <SettingRow
                    id={NOTIFICATIONS_SETTINGS.quietHours.id}
                    indent
                    label={NOTIFICATIONS_SETTINGS.quietHours.label}
                    description={NOTIFICATIONS_SETTINGS.quietHours.description}
                    control={
                        <input
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
                        <select
                            class="setting-select"
                            value={(s()["notify:os:preview"] as string) ?? "redacted"}
                            onChange={(e) => set("notify:os:preview", e.currentTarget.value)}
                        >
                            <option value="redacted">Short preview</option>
                            <option value="full">Longer preview</option>
                            <option value="none">Title only</option>
                        </select>
                    }
                />
            </Show>
            <SettingRow
                id={NOTIFICATIONS_SETTINGS.osTest.id}
                label={NOTIFICATIONS_SETTINGS.osTest.label}
                description={NOTIFICATIONS_SETTINGS.osTest.description}
                control={
                    <button
                        type="button"
                        class="setting-masked-key-btn"
                        onClick={() => void RpcApi.NotifyTestCommand(TabRpcClient).catch(() => {})}
                    >
                        Send
                    </button>
                }
            />
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
