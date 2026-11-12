// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { settingEntry, type SettingsIndexEntry } from "../settings-model";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const SOUNDS_SETTINGS = {
    notificationSoundsEnabled: settingEntry({
        id: "sounds.notifications_enabled",
        labelKey: "settings.sounds.notificationSoundsEnabled.label",
        descriptionKey: "settings.sounds.notificationSoundsEnabled.description",
        section: "sounds",
        keywords: ["sound effects", "audio alerts", "mute sounds", "notify:sounds:enabled"],
    }),
    volume: settingEntry({
        id: "sounds.volume",
        labelKey: "settings.sounds.volume.label",
        section: "sounds",
        keywords: ["sound volume", "notification volume", "notify:sounds:volume"],
    }),
    suppressWhenFocused: settingEntry({
        id: "sounds.suppress_when_focused",
        labelKey: "settings.sounds.suppressWhenFocused.label",
        descriptionKey: "settings.sounds.suppressWhenFocused.description",
        section: "sounds",
        keywords: ["mute active pane", "focused pane sound", "notify:sounds:suppresswhenfocused"],
    }),
    turnComplete: settingEntry({
        id: "sounds.turn_complete",
        labelKey: "settings.sounds.turnComplete.label",
        descriptionKey: "settings.sounds.turnComplete.description",
        section: "sounds",
        keywords: ["done sound", "finished sound", "notify:sound:agent.turn.complete"],
    }),
    turnError: settingEntry({
        id: "sounds.turn_error",
        labelKey: "settings.sounds.turnError.label",
        descriptionKey: "settings.sounds.turnError.description",
        section: "sounds",
        keywords: ["error sound", "failure sound", "notify:sound:agent.turn.error"],
    }),
    turnInterrupted: settingEntry({
        id: "sounds.turn_interrupted",
        labelKey: "settings.sounds.turnInterrupted.label",
        descriptionKey: "settings.sounds.turnInterrupted.description",
        section: "sounds",
        keywords: ["stop sound", "interrupt sound", "notify:sound:agent.turn.interrupted"],
    }),
    messageAccepted: settingEntry({
        id: "sounds.message_accepted",
        labelKey: "settings.sounds.messageAccepted.label",
        descriptionKey: "settings.sounds.messageAccepted.description",
        section: "sounds",
        keywords: ["queued message sound", "notify:sound:agent.message.accepted"],
    }),
    messageRejected: settingEntry({
        id: "sounds.message_rejected",
        labelKey: "settings.sounds.messageRejected.label",
        descriptionKey: "settings.sounds.messageRejected.description",
        section: "sounds",
        keywords: ["queued message sound", "notify:sound:agent.message.rejected"],
    }),
    toolTonesEnabled: settingEntry({
        id: "sounds.tool_tones_enabled",
        labelKey: "settings.sounds.toolTonesEnabled.label",
        descriptionKey: "settings.sounds.toolTonesEnabled.description",
        section: "sounds",
        groupKey: "settings.sounds.group.toolCallTones",
        keywords: ["tool call tones", "tool call sound", "synth tone", "notify:tooltones:enabled"],
    }),
    toolTonesVolume: settingEntry({
        id: "sounds.tool_tones_volume",
        labelKey: "settings.sounds.toolTonesVolume.label",
        section: "sounds",
        groupKey: "settings.sounds.group.toolCallTones",
        keywords: ["tool tone volume", "notify:tooltones:volume"],
    }),
    toolTonesScope: settingEntry({
        id: "sounds.tool_tones_scope",
        labelKey: "settings.sounds.toolTonesScope.label",
        descriptionKey: "settings.sounds.toolTonesScope.description",
        section: "sounds",
        groupKey: "settings.sounds.group.toolCallTones",
        keywords: ["tool tone scope", "focused only", "all panes", "notify:tooltones:scope"],
    }),
    toolTonesFlash: settingEntry({
        id: "sounds.tool_tones_flash",
        labelKey: "settings.sounds.toolTonesFlash.label",
        descriptionKey: "settings.sounds.toolTonesFlash.description",
        section: "sounds",
        keywords: ["activity flash", "tab flash", "flash with sounds", "highlight tab", "which pane", "notify:tooltones:flash"],
    }),
    waitingToneEnabled: settingEntry({
        id: "sounds.waiting_tone_enabled",
        labelKey: "settings.sounds.waitingToneEnabled.label",
        descriptionKey: "settings.sounds.waitingToneEnabled.description",
        section: "sounds",
        groupKey: "settings.sounds.group.waitingForInput",
        keywords: ["waiting for input sound", "ambient tone", "blocked sound", "permission", "approval", "notify:sound:agent.waiting.for.input"],
    }),
    waitingBrowser: settingEntry({
        id: "sounds.waiting_browser",
        labelKey: "settings.sounds.waitingBrowser.label",
        descriptionKey: "settings.sounds.waitingBrowser.description",
        section: "sounds",
        groupKey: "settings.sounds.group.waitingForInput",
        keywords: ["browser", "hand-off", "handoff", "approval", "notify:waiting:browser"],
    }),
    waitingToast: settingEntry({
        id: "sounds.waiting_toast",
        labelKey: "settings.sounds.waitingToast.label",
        descriptionKey: "settings.sounds.waitingToast.description",
        section: "sounds",
        groupKey: "settings.sounds.group.waitingForInput",
        keywords: ["toast", "notification", "link", "go to agent", "attention", "notify:waiting:toast"],
    }),
    shutdownToneEnabled: settingEntry({
        id: "sounds.shutdown_tone_enabled",
        labelKey: "settings.sounds.shutdownToneEnabled.label",
        descriptionKey: "settings.sounds.shutdownToneEnabled.description",
        section: "sounds",
        groupKey: "settings.sounds.group.shutdownWarning",
        keywords: ["shutdown sound", "quit warning", "keep running", "notify:sound:agent.shutdown.pending"],
    }),
    waitingToneVolume: settingEntry({
        id: "sounds.waiting_tone_volume",
        labelKey: "settings.sounds.waitingToneVolume.label",
        section: "sounds",
        groupKey: "settings.sounds.group.waitingForInput",
        keywords: ["waiting tone volume", "notify:sounds:waiting:volume"],
    }),
} satisfies Record<string, SettingsIndexEntry>;
