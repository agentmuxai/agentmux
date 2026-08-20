// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { KeyValueEditor, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";

// ── Section: Advanced ─────────────────────────────────────────────────────────

export function AdvancedSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);

    return (
        <div class="settings-section-body">
            <SectionHeader label="Terminal (power user)" />
            <SettingRow
                label="Disable WebGL rendering"
                description="Fall back to canvas-based terminal rendering (restart required)"
                control={
                    <ToggleControl
                        checked={!!(s()["term:disablewebgl"] as boolean)}
                        onChange={(v) => set("term:disablewebgl", v)}
                    />
                }
            />
            <SectionHeader label="Agent panes" />
            <SettingRow
                label="Auto-answer timeout"
                description="Seconds an AskUserQuestion panel waits for you before auto-selecting the recommended option(s)"
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={1}
                        value={((s()["agent:askquestiontimeoutms"] as number) ?? 30000) / 1000}
                        onBlur={(e) => {
                            const v = parseFloat(e.currentTarget.value);
                            if (!isNaN(v) && v >= 1) set("agent:askquestiontimeoutms", Math.round(v * 1000));
                        }}
                    />
                }
            />
            <SectionHeader label="Widgets" />
            <SettingRow
                label="Icon-only widget labels"
                description="Force the widget bar to show icons without text labels"
                control={
                    <ToggleControl
                        checked={!!(s()["widget:icononly"] as boolean)}
                        onChange={(v) => set("widget:icononly", v)}
                    />
                }
            />
            <SectionHeader label="Sysinfo widget" />
            <SettingRow
                label="Sample interval"
                description="Seconds between sysinfo widget samples"
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={1}
                        value={(s()["telemetry:interval"] as number) ?? 1}
                        onBlur={(e) => {
                            const v = parseFloat(e.currentTarget.value);
                            if (!isNaN(v) && v >= 1) set("telemetry:interval", v);
                        }}
                    />
                }
            />
            <SettingRow
                label="History length"
                description="Number of sysinfo widget samples retained (30–1024)"
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={30} max={1024}
                        value={(s()["telemetry:numpoints"] as number) ?? 120}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 30 && v <= 1024) set("telemetry:numpoints", v);
                        }}
                    />
                }
            />
            <SectionHeader label="Environment" />
            <SettingRow
                stacked
                label="Global environment variables"
                description="Environment variables injected into every shell"
                control={
                    <KeyValueEditor
                        value={(s()["cmd:env"] as Record<string, string>) ?? {}}
                        onChange={(v) => set("cmd:env", v)}
                    />
                }
            />
        </div>
    );
}
