// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The groups of Settings → Sounds & Notifications (sounds-section.tsx lays them out): the notification sounds, the
// tool-call tones, the waiting tone and the shutdown warning. Each group is its rows, returned as a fragment.

import { Select } from "@/app/element/ui";
import { Show, type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { SectionHeader, set, SettingRow, SliderControl, ToggleControl } from "../settings-controls";
import { DEFAULT_MASTER_VOLUME, DEFAULT_TOOLTONES_VOLUME } from "@/app/notification/sound/sound-defaults";
import { SOUNDS_SETTINGS } from "./sounds-settings-index";

const s = () => settingsAtom() ?? ({} as any);

/** One sound per event: turn complete, error and interrupted, and a queued message accepted or rejected. */
function EventSounds(): JSX.Element {
    return (
        <>
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
        </>
    );
}

/** The master switch for notification sounds, and what it governs: volume, suppression while focused, the flash and one sound per event. */
export function NotificationSoundsGroup(): JSX.Element {
    const soundsEnabled = () => s()["notify:sounds:enabled"] !== false;
    return (
        <>
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
                {/* One toggle for every sound's flash, tool tones and event
                    sounds alike, so it lives here rather than under tool-call
                    tones (where turning tones off would hide it).
                    SPEC_AGENT_ACTIVITY_FLASH_SOUND_SYNC_2026_09_24.md §3.7. */}
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
                <EventSounds />
            </Show>
        </>
    );
}

/** The tool-call tones: their switch, volume and scope. */
export function ToolTonesGroup(): JSX.Element {
    const toolTonesEnabled = () => s()["notify:tooltones:enabled"] !== false;
    return (
        <>
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
                        <Select
                            class="setting-select"
                            value={(s()["notify:tooltones:scope"] as string) ?? "all"}
                            onChange={(v) => set("notify:tooltones:scope", v)}
                        >
                            <option value="all">All panes</option>
                            <option value="focused">Focused pane only</option>
                        </Select>
                    }
                />
            </Show>
        </>
    );
}

/** The looping tone while an agent waits for the person: its switch, volume and the browser hand-offs. */
export function WaitingToneGroup(): JSX.Element {
    const waitingToneEnabled = () => s()["notify:sound:agent.waiting.for.input"] !== false;
    return (
        <>
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
                <SettingRow
                    id={SOUNDS_SETTINGS.waitingBrowser.id}
                    indent
                    label={SOUNDS_SETTINGS.waitingBrowser.label}
                    description={SOUNDS_SETTINGS.waitingBrowser.description}
                    control={
                        <ToggleControl
                            checked={s()["notify:waiting:browser"] !== false}
                            onChange={(v) => set("notify:waiting:browser", v)}
                        />
                    }
                />
            </Show>
        </>
    );
}

/** The chime when something other than the person asks to shut an agent down. */
export function ShutdownWarningGroup(): JSX.Element {
    return (
        <>
            <SectionHeader label="Shutdown warning" />
            <SettingRow
                id={SOUNDS_SETTINGS.shutdownToneEnabled.id}
                label={SOUNDS_SETTINGS.shutdownToneEnabled.label}
                description={SOUNDS_SETTINGS.shutdownToneEnabled.description}
                control={
                    <ToggleControl
                        checked={s()["notify:sound:agent.shutdown.pending"] !== false}
                        onChange={(v) => set("notify:sound:agent.shutdown.pending", v)}
                    />
                }
            />
        </>
    );
}
