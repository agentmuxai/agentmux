// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { SectionHeader, set, SettingRow, SliderControl, ToggleControl } from "../settings-controls";

// ── Section: Sounds & Notifications ───────────────────────────────────────────

export function SoundsSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const soundsEnabled = () => s()["notify:sounds:enabled"] !== false;
    const toolTonesEnabled = () => s()["notify:tooltones:enabled"] !== false;
    const waitingToneEnabled = () => s()["notify:sound:agent.waiting.for.input"] !== false;

    return (
        <div class="settings-section-body">
            <SettingRow
                label="Notification sounds"
                description="Master enable for notification sounds"
                control={<ToggleControl checked={soundsEnabled()} onChange={(v) => set("notify:sounds:enabled", v)} />}
            />
            <Show when={soundsEnabled()}>
                <SettingRow
                    indent
                    label="Volume"
                    control={
                        <SliderControl
                            min={0} max={1} step={0.05}
                            value={(s()["notify:sounds:volume"] as number) ?? 0.6}
                            onChange={(v) => set("notify:sounds:volume", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Suppress when focused"
                    description="Don't play a pane's sound when it's already focused and visible"
                    control={
                        <ToggleControl
                            checked={s()["notify:sounds:suppresswhenfocused"] !== false}
                            onChange={(v) => set("notify:sounds:suppresswhenfocused", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Turn complete"
                    description="Play a sound when an agent turn completes normally"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.complete"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.complete", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Turn error"
                    description="Play a sound when an agent turn ends with an error"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.error"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.error", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Turn interrupted"
                    description="Play a sound when an agent turn is stopped or interrupted"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.turn.interrupted"] !== false}
                            onChange={(v) => set("notify:sound:agent.turn.interrupted", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Message accepted"
                    description="Play a sound when a queued pending message is accepted"
                    control={
                        <ToggleControl
                            checked={s()["notify:sound:agent.message.accepted"] !== false}
                            onChange={(v) => set("notify:sound:agent.message.accepted", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Message rejected"
                    description="Play a sound when a queued pending message is rejected"
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
                label="Enable"
                description="Play a subliminal synth tone for every agent tool call"
                control={<ToggleControl checked={toolTonesEnabled()} onChange={(v) => set("notify:tooltones:enabled", v)} />}
            />
            <Show when={toolTonesEnabled()}>
                <SettingRow
                    indent
                    label="Volume"
                    control={
                        <SliderControl
                            min={0} max={1} step={0.05}
                            value={(s()["notify:tooltones:volume"] as number) ?? 0.15}
                            onChange={(v) => set("notify:tooltones:volume", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Scope"
                    description="Which panes play tool-call tones"
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
            </Show>
            <SectionHeader label="Waiting for input" />
            <SettingRow
                label="Enable"
                description="Play a looping ambient tone while an agent pane is blocked waiting for your input"
                control={<ToggleControl checked={waitingToneEnabled()} onChange={(v) => set("notify:sound:agent.waiting.for.input", v)} />}
            />
            <Show when={waitingToneEnabled()}>
                <SettingRow
                    indent
                    label="Volume"
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
