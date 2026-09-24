// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader, set, SettingRow, SliderControl, ToggleControl } from "../settings-controls";
import { DEFAULT_MASTER_VOLUME, DEFAULT_TOOLTONES_VOLUME } from "@/app/notification/sound/sound-defaults";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const SOUNDS_SETTINGS = {
    notificationSoundsEnabled: {
        id: "sounds.notifications_enabled",
        label: "Notification sounds",
        description: "Master enable for notification sounds",
        section: "sounds",
        keywords: ["sound effects", "audio alerts", "mute sounds", "notify:sounds:enabled"],
    },
    volume: {
        id: "sounds.volume",
        label: "Volume",
        section: "sounds",
        keywords: ["sound volume", "notification volume", "notify:sounds:volume"],
    },
    suppressWhenFocused: {
        id: "sounds.suppress_when_focused",
        label: "Suppress when focused",
        description: "Don't play a pane's sound when it's already focused and visible",
        section: "sounds",
        keywords: ["mute active pane", "focused pane sound", "notify:sounds:suppresswhenfocused"],
    },
    turnComplete: {
        id: "sounds.turn_complete",
        label: "Turn complete",
        description: "Play a sound when an agent turn completes normally",
        section: "sounds",
        keywords: ["done sound", "finished sound", "notify:sound:agent.turn.complete"],
    },
    turnError: {
        id: "sounds.turn_error",
        label: "Turn error",
        description: "Play a sound when an agent turn ends with an error",
        section: "sounds",
        keywords: ["error sound", "failure sound", "notify:sound:agent.turn.error"],
    },
    turnInterrupted: {
        id: "sounds.turn_interrupted",
        label: "Turn interrupted",
        description: "Play a sound when an agent turn is stopped or interrupted",
        section: "sounds",
        keywords: ["stop sound", "interrupt sound", "notify:sound:agent.turn.interrupted"],
    },
    messageAccepted: {
        id: "sounds.message_accepted",
        label: "Message accepted",
        description: "Play a sound when a queued pending message is accepted",
        section: "sounds",
        keywords: ["queued message sound", "notify:sound:agent.message.accepted"],
    },
    messageRejected: {
        id: "sounds.message_rejected",
        label: "Message rejected",
        description: "Play a sound when a queued pending message is rejected",
        section: "sounds",
        keywords: ["queued message sound", "notify:sound:agent.message.rejected"],
    },
    toolTonesEnabled: {
        id: "sounds.tool_tones_enabled",
        label: "Enable",
        description: "Play a subliminal synth tone for every agent tool call",
        section: "sounds",
        keywords: ["tool call tones", "tool call sound", "synth tone", "notify:tooltones:enabled"],
    },
    toolTonesVolume: {
        id: "sounds.tool_tones_volume",
        label: "Volume",
        section: "sounds",
        keywords: ["tool tone volume", "notify:tooltones:volume"],
    },
    toolTonesScope: {
        id: "sounds.tool_tones_scope",
        label: "Scope",
        description: "Which panes play tool-call tones",
        section: "sounds",
        keywords: ["tool tone scope", "focused only", "all panes", "notify:tooltones:scope"],
    },
    toolTonesFlash: {
        id: "sounds.tool_tones_flash",
        label: "Flash source tab",
        description: "Briefly flash the window tab and pane tab each tone came from, in that pane's color",
        section: "sounds",
        keywords: ["activity flash", "tab flash", "highlight tab", "which pane", "notify:tooltones:flash"],
    },
    waitingToneEnabled: {
        id: "sounds.waiting_tone_enabled",
        label: "Enable",
        description: "Play a looping ambient tone while an agent pane is blocked waiting for your input",
        section: "sounds",
        keywords: ["waiting for input sound", "ambient tone", "blocked sound", "notify:sound:agent.waiting.for.input"],
    },
    waitingToneVolume: {
        id: "sounds.waiting_tone_volume",
        label: "Volume",
        section: "sounds",
        keywords: ["waiting tone volume", "notify:sounds:waiting:volume"],
    },
} satisfies Record<string, SettingsIndexEntry>;

// ── Section: Sounds & Notifications ───────────────────────────────────────────

export function SoundsSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const soundsEnabled = () => s()["notify:sounds:enabled"] !== false;
    const toolTonesEnabled = () => s()["notify:tooltones:enabled"] !== false;
    const waitingToneEnabled = () => s()["notify:sound:agent.waiting.for.input"] !== false;

    return (
        <div class="settings-section-body">
            <SettingRow
                id={SOUNDS_SETTINGS.notificationSoundsEnabled.id}
                label={SOUNDS_SETTINGS.notificationSoundsEnabled.label}
                description={SOUNDS_SETTINGS.notificationSoundsEnabled.description}
                control={<ToggleControl checked={soundsEnabled()} onChange={(v) => set("notify:sounds:enabled", v)} />}
            />
            <Show when={soundsEnabled()}>
                <SettingRow
                    id={SOUNDS_SETTINGS.volume.id}
                    indent
                    label={SOUNDS_SETTINGS.volume.label}
                    control={
                        <SliderControl
                            min={0} max={1} step={0.05}
                            value={(s()["notify:sounds:volume"] as number) ?? DEFAULT_MASTER_VOLUME}
                            onChange={(v) => set("notify:sounds:volume", v)}
                        />
                    }
                />
                <SettingRow
                    id={SOUNDS_SETTINGS.suppressWhenFocused.id}
                    indent
                    label={SOUNDS_SETTINGS.suppressWhenFocused.label}
                    description={SOUNDS_SETTINGS.suppressWhenFocused.description}
                    control={
                        <ToggleControl
                            checked={s()["notify:sounds:suppresswhenfocused"] !== false}
                            onChange={(v) => set("notify:sounds:suppresswhenfocused", v)}
                        />
                    }
                />
                <SettingRow
                    id={SOUNDS_SETTINGS.turnComplete.id}
                    indent
                    label={SOUNDS_SETTINGS.turnComplete.label}
                    description={SOUNDS_SETTINGS.turnComplete.description}
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.complete"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.complete", v)}
                        />
                    }
                />
                <SettingRow
                    id={SOUNDS_SETTINGS.turnError.id}
                    indent
                    label={SOUNDS_SETTINGS.turnError.label}
                    description={SOUNDS_SETTINGS.turnError.description}
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.error"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.error", v)}
                        />
                    }
                />
                <SettingRow
                    id={SOUNDS_SETTINGS.turnInterrupted.id}
                    indent
                    label={SOUNDS_SETTINGS.turnInterrupted.label}
                    description={SOUNDS_SETTINGS.turnInterrupted.description}
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.interrupted"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.interrupted", v)}
                        />
                    }
                />
                <SettingRow
                    id={SOUNDS_SETTINGS.messageAccepted.id}
                    indent
                    label={SOUNDS_SETTINGS.messageAccepted.label}
                    description={SOUNDS_SETTINGS.messageAccepted.description}
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.message.accepted"] !== false}
                            onChange={(v) => set("notify:sound:agent.message.accepted", v)}
                        />
                    }
                />
                <SettingRow
                    id={SOUNDS_SETTINGS.messageRejected.id}
                    indent
                    label={SOUNDS_SETTINGS.messageRejected.label}
                    description={SOUNDS_SETTINGS.messageRejected.description}
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.message.rejected"] !== false}
                            onChange={(v) => set("notify:sound:agent.message.rejected", v)}
                        />
                    }
                />
            </Show>
            <SectionHeader label="Tool-call tones" />
            <SettingRow
                id={SOUNDS_SETTINGS.toolTonesEnabled.id}
                label={SOUNDS_SETTINGS.toolTonesEnabled.label}
                description={SOUNDS_SETTINGS.toolTonesEnabled.description}
                control={<ToggleControl checked={toolTonesEnabled()} onChange={(v) => set("notify:tooltones:enabled", v)} />}
            />
            <Show when={toolTonesEnabled()}>
                <SettingRow
                    id={SOUNDS_SETTINGS.toolTonesVolume.id}
                    indent
                    label={SOUNDS_SETTINGS.toolTonesVolume.label}
                    control={
                        <SliderControl
                            min={0} max={1} step={0.05}
                            value={(s()["notify:tooltones:volume"] as number) ?? DEFAULT_TOOLTONES_VOLUME}
                            onChange={(v) => set("notify:tooltones:volume", v)}
                        />
                    }
                />
                <SettingRow
                    id={SOUNDS_SETTINGS.toolTonesScope.id}
                    indent
                    label={SOUNDS_SETTINGS.toolTonesScope.label}
                    description={SOUNDS_SETTINGS.toolTonesScope.description}
                    control={
                        <select
                            class="setting-select"
                            value={(s()["notify:tooltones:scope"] as string) ?? "all"}
                            onChange={(e) => set("notify:tooltones:scope", e.currentTarget.value)}
                        >
                            <option value="all">All panes</option>
                            <option value="focused">Focused pane only</option>
                        </select>
                    }
                />
                <SettingRow
                    id={SOUNDS_SETTINGS.toolTonesFlash.id}
                    indent
                    label={SOUNDS_SETTINGS.toolTonesFlash.label}
                    description={SOUNDS_SETTINGS.toolTonesFlash.description}
                    control={
                        <ToggleControl
                            checked={s()["notify:tooltones:flash"] !== false}
                            onChange={(v) => set("notify:tooltones:flash", v)}
                        />
                    }
                />
            </Show>
            <SectionHeader label="Waiting for input" />
            <SettingRow
                id={SOUNDS_SETTINGS.waitingToneEnabled.id}
                label={SOUNDS_SETTINGS.waitingToneEnabled.label}
                description={SOUNDS_SETTINGS.waitingToneEnabled.description}
                control={<ToggleControl checked={waitingToneEnabled()} onChange={(v) => set("notify:sound:agent.waiting.for.input", v)} />}
            />
            <Show when={waitingToneEnabled()}>
                <SettingRow
                    id={SOUNDS_SETTINGS.waitingToneVolume.id}
                    indent
                    label={SOUNDS_SETTINGS.waitingToneVolume.label}
                    control={
                        <SliderControl
                            min={0} max={1} step={0.05}
                            value={(s()["notify:sounds:waiting:volume"] as number) ?? 0.25}
                            onChange={(v) => set("notify:sounds:waiting:volume", v)}
                        />
                    }
                />
            </Show>
        </div>
    );
}
