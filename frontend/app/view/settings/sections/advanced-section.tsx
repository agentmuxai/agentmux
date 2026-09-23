// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import type { SettingsIndexEntry } from "../settings-model";
import { KeyValueEditor, NumberControl, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const ADVANCED_SETTINGS = {
    disableWebgl: {
        id: "advanced.disable_webgl",
        label: "Disable WebGL rendering",
        description: "Fall back to canvas-based terminal rendering (restart required)",
        section: "advanced",
        keywords: ["gpu rendering", "canvas fallback", "graphics glitch", "term:disablewebgl"],
    },
    autoAnswerTimeout: {
        id: "advanced.auto_answer_timeout",
        label: "Auto-answer timeout",
        description: "Seconds an AskUserQuestion panel waits for you before auto-selecting the recommended option(s)",
        section: "advanced",
        keywords: ["auto select", "question timeout", "agent:askquestiontimeoutms"],
    },
    iconOnlyWidgetLabels: {
        id: "advanced.icon_only_widget_labels",
        label: "Icon-only widget labels",
        description: "Force the widget bar to show icons without text labels",
        section: "advanced",
        keywords: ["compact widget bar", "hide labels", "widget:icononly"],
    },
    sampleInterval: {
        id: "advanced.sysinfo_sample_interval",
        label: "Sample interval",
        description: "Seconds between sysinfo widget samples",
        section: "advanced",
        keywords: ["sysinfo refresh rate", "polling interval", "telemetry:interval"],
    },
    historyLength: {
        id: "advanced.sysinfo_history_length",
        label: "History length",
        description: "Number of sysinfo widget samples retained (30–1024)",
        section: "advanced",
        keywords: ["sysinfo history", "graph length", "telemetry:numpoints"],
    },
    enableFileDrop: {
        id: "advanced.enable_file_drop",
        label: "Enable file drop",
        description: "Drag files onto an agent pane to attach them",
        section: "advanced",
        keywords: ["drag and drop", "file attach", "dnd:enabled"],
    },
    insertReferenceToken: {
        id: "advanced.insert_reference_token",
        label: "Insert reference token",
        description: "Also insert a file-reference token into the composer on drop, in addition to attaching the file",
        section: "advanced",
        keywords: ["file reference", "drop token", "dnd:agentinserttoken"],
    },
    maxConcurrentUploads: {
        id: "advanced.max_concurrent_uploads",
        label: "Max concurrent uploads",
        description: "Files uploaded at once on a multi-file drop. Leave blank for unlimited.",
        section: "advanced",
        keywords: ["upload concurrency", "parallel uploads", "dnd:concurrency"],
    },
    globalEnvVars: {
        id: "advanced.global_env_vars",
        label: "Global environment variables",
        description: "Environment variables injected into every shell",
        section: "advanced",
        keywords: ["environment variables", "env vars", "shell env", "cmd:env"],
    },
} satisfies Record<string, SettingsIndexEntry>;

// ── Section: Advanced ─────────────────────────────────────────────────────────

export function AdvancedSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);

    return (
        <div class="settings-section-body">
            <SectionHeader label="Terminal (power user)" />
            <SettingRow
                id={ADVANCED_SETTINGS.disableWebgl.id}
                label={ADVANCED_SETTINGS.disableWebgl.label}
                description={ADVANCED_SETTINGS.disableWebgl.description}
                control={
                    <ToggleControl
                        checked={!!(s()["term:disablewebgl"] as boolean)}
                        onChange={(v) => set("term:disablewebgl", v)}
                    />
                }
            />
            <SectionHeader label="Agent panes" />
            <SettingRow
                id={ADVANCED_SETTINGS.autoAnswerTimeout.id}
                label={ADVANCED_SETTINGS.autoAnswerTimeout.label}
                description={ADVANCED_SETTINGS.autoAnswerTimeout.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={1} step={1}
                        value={((s()["agent:askquestiontimeoutms"] as number) ?? 30000) / 1000}
                        onChange={(v) => set("agent:askquestiontimeoutms", Math.round(v * 1000))}
                    />
                }
            />
            <SectionHeader label="Widgets" />
            <SettingRow
                id={ADVANCED_SETTINGS.iconOnlyWidgetLabels.id}
                label={ADVANCED_SETTINGS.iconOnlyWidgetLabels.label}
                description={ADVANCED_SETTINGS.iconOnlyWidgetLabels.description}
                control={
                    <ToggleControl
                        checked={!!(s()["widget:icononly"] as boolean)}
                        onChange={(v) => set("widget:icononly", v)}
                    />
                }
            />
            <SectionHeader label="Sysinfo widget" />
            <SettingRow
                id={ADVANCED_SETTINGS.sampleInterval.id}
                label={ADVANCED_SETTINGS.sampleInterval.label}
                description={ADVANCED_SETTINGS.sampleInterval.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={1} step={1}
                        value={(s()["telemetry:interval"] as number) ?? 1}
                        onChange={(v) => set("telemetry:interval", v)}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.historyLength.id}
                label={ADVANCED_SETTINGS.historyLength.label}
                description={ADVANCED_SETTINGS.historyLength.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={30} max={1024} step={1} parse="int"
                        value={(s()["telemetry:numpoints"] as number) ?? 120}
                        onChange={(v) => set("telemetry:numpoints", v)}
                    />
                }
            />
            <SectionHeader label="Drag & drop" />
            <SettingRow
                id={ADVANCED_SETTINGS.enableFileDrop.id}
                label={ADVANCED_SETTINGS.enableFileDrop.label}
                description={ADVANCED_SETTINGS.enableFileDrop.description}
                control={
                    <ToggleControl
                        checked={(s()["dnd:enabled"] as boolean) ?? true}
                        onChange={(v) => set("dnd:enabled", v)}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.insertReferenceToken.id}
                label={ADVANCED_SETTINGS.insertReferenceToken.label}
                description={ADVANCED_SETTINGS.insertReferenceToken.description}
                control={
                    <ToggleControl
                        checked={(s()["dnd:agentinserttoken"] as boolean) ?? true}
                        onChange={(v) => set("dnd:agentinserttoken", v)}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.maxConcurrentUploads.id}
                label={ADVANCED_SETTINGS.maxConcurrentUploads.label}
                description={ADVANCED_SETTINGS.maxConcurrentUploads.description}
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={1}
                        placeholder="unlimited"
                        value={(s()["dnd:concurrency"] as number) ?? ""}
                        onBlur={(e) => {
                            const raw = e.currentTarget.value.trim();
                            if (raw === "") { set("dnd:concurrency", null); return; }
                            const v = parseInt(raw, 10);
                            if (!isNaN(v) && v >= 1) set("dnd:concurrency", v);
                        }}
                    />
                }
            />
            <SectionHeader label="Environment" />
            <SettingRow
                id={ADVANCED_SETTINGS.globalEnvVars.id}
                stacked
                label={ADVANCED_SETTINGS.globalEnvVars.label}
                description={ADVANCED_SETTINGS.globalEnvVars.description}
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
