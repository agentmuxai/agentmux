// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The groups of Settings → Advanced (advanced-section.tsx lays them out): Terminal, Agent panes in three runs,
// Sysinfo widget, Drag & drop, Environment and Developer. Each group is its header and its rows.

import { TextInput } from "@/app/element/ui";
import { type JSX } from "solid-js";

import { devModeOn, setDevMode } from "@/app/store/dev-mode";
import { settingsAtom } from "@/app/store/global";
import { KANBAN_REMINDERS_SETTING, kanbanRemindersOn } from "@/app/view/kanban/kanban-reminders";
import { resolveSkidNotches, SKID_NOTCHES_MAX, SKID_NOTCHES_SETTING } from "@/app/view/agent/components/scroll-handoff";
import { KeyValueEditor, NumberControl, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";
import { ADVANCED_SETTINGS } from "./advanced-settings-index";

const s = () => settingsAtom() ?? ({} as any);

/** A limit setting: "auto" (shown as it is, stored as unset), 0 for no limit, or a positive number. */
function LimitInput(props: { settingKey: "agent:container_memory" | "agent:container_cpus"; value: unknown }): JSX.Element {
    return (
        <TextInput
            class="setting-number setting-number--wide"
            placeholder="auto"
            value={props.value == null ? "auto" : String(props.value)}
            onBlur={(e) => {
                const raw = e.currentTarget.value.trim().toLowerCase();
                if (raw === "" || raw === "auto") {
                    set(props.settingKey, null);
                    return;
                }
                const n = Number(raw);
                if (Number.isFinite(n) && n >= 0) set(props.settingKey, n);
                else e.currentTarget.value = props.value == null ? "auto" : String(props.value);
            }}
        />
    );
}

/** Terminal (power user). */
export function TerminalGroup(): JSX.Element {
    return (
        <>
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
        </>
    );
}

/** Agent panes: how long a question waits, when to compact, the scroll skid, and the container limits. */
export function AgentPaneLimits(): JSX.Element {
    return (
        <>
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
            <SettingRow
                id={ADVANCED_SETTINGS.compactAt.id}
                label={ADVANCED_SETTINGS.compactAt.label}
                description={ADVANCED_SETTINGS.compactAt.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={0} step={50}
                        value={((s()["agent:compactat"] as number) ?? 600000) / 1000}
                        onChange={(v) => set("agent:compactat", Math.max(0, Math.round(v)) * 1000)}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.previewSkidNotches.id}
                label={ADVANCED_SETTINGS.previewSkidNotches.label}
                description={ADVANCED_SETTINGS.previewSkidNotches.description}
                control={
                    <NumberControl
                        class="setting-number"
                        min={0}
                        max={SKID_NOTCHES_MAX}
                        step={1}
                        parse="int"
                        value={resolveSkidNotches(s()[SKID_NOTCHES_SETTING])}
                        onChange={(v) => set(SKID_NOTCHES_SETTING, Math.max(0, Math.min(SKID_NOTCHES_MAX, Math.round(v))))}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.containerMemory.id}
                label={ADVANCED_SETTINGS.containerMemory.label}
                description={ADVANCED_SETTINGS.containerMemory.description}
                control={<LimitInput settingKey="agent:container_memory" value={s()["agent:container_memory"]} />}
            />
            <SettingRow
                id={ADVANCED_SETTINGS.containerCpus.id}
                label={ADVANCED_SETTINGS.containerCpus.label}
                description={ADVANCED_SETTINGS.containerCpus.description}
                control={<LimitInput settingKey="agent:container_cpus" value={s()["agent:container_cpus"]} />}
            />
        </>
    );
}

/** Agent panes: the sub-agent and Explore models. */
export function AgentPaneModels(): JSX.Element {
    return (
        <>
            <SettingRow
                id={ADVANCED_SETTINGS.subagentModel.id}
                label={ADVANCED_SETTINGS.subagentModel.label}
                description={ADVANCED_SETTINGS.subagentModel.description}
                control={
                    <TextInput
                        class="setting-number setting-number--wide"
                        placeholder="sonnet"
                        value={(s()["agent:subagentmodel"] as string) ?? "sonnet"}
                        onBlur={(e) => {
                            const raw = e.currentTarget.value.trim();
                            set("agent:subagentmodel", raw === "" ? null : raw);
                        }}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.exploreModel.id}
                label={ADVANCED_SETTINGS.exploreModel.label}
                description={ADVANCED_SETTINGS.exploreModel.description}
                control={
                    <TextInput
                        class="setting-number setting-number--wide"
                        placeholder="haiku"
                        value={(s()["agent:exploremodel"] as string) ?? "haiku"}
                        onBlur={(e) => {
                            const raw = e.currentTarget.value.trim();
                            set("agent:exploremodel", raw === "" ? null : raw);
                        }}
                    />
                }
            />
        </>
    );
}

/** Agent panes: compact hints, overlap notes, retry after reconnect, and backlog reminders. */
export function AgentPaneToggles(): JSX.Element {
    return (
        <>
            <SettingRow
                id={ADVANCED_SETTINGS.compactHints.id}
                label={ADVANCED_SETTINGS.compactHints.label}
                description={ADVANCED_SETTINGS.compactHints.description}
                control={
                    <ToggleControl
                        checked={(s()["agent:compacthints"] as boolean) ?? true}
                        onChange={(v) => set("agent:compacthints", v)}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.overlapNotes.id}
                label={ADVANCED_SETTINGS.overlapNotes.label}
                description={ADVANCED_SETTINGS.overlapNotes.description}
                control={
                    <ToggleControl
                        checked={(s()["agent:overlapnotes"] as boolean) ?? true}
                        onChange={(v) => set("agent:overlapnotes", v)}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.retryOnReconnect.id}
                label={ADVANCED_SETTINGS.retryOnReconnect.label}
                description={ADVANCED_SETTINGS.retryOnReconnect.description}
                control={
                    <ToggleControl
                        checked={(s()["agent:retryonreconnect"] as boolean) ?? true}
                        onChange={(v) => set("agent:retryonreconnect", v)}
                    />
                }
            />
            <SettingRow
                id={ADVANCED_SETTINGS.kanbanReminders.id}
                label={ADVANCED_SETTINGS.kanbanReminders.label}
                description={ADVANCED_SETTINGS.kanbanReminders.description}
                control={
                    <ToggleControl
                        checked={kanbanRemindersOn(s())}
                        onChange={(v) => set(KANBAN_REMINDERS_SETTING, v)}
                    />
                }
            />
        </>
    );
}

/** The sysinfo widget. */
export function SysinfoGroup(): JSX.Element {
    return (
        <>
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
        </>
    );
}

/** Drag & drop. */
export function DragDropGroup(): JSX.Element {
    return (
        <>
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
                    <TextInput
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
        </>
    );
}

/** Environment. */
export function EnvironmentGroup(): JSX.Element {
    return (
        <>
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
        </>
    );
}

/** Developer. */
export function DeveloperGroup(): JSX.Element {
    return (
        <>
            <SectionHeader label="Developer" />
            <SettingRow
                id={ADVANCED_SETTINGS.devMode.id}
                label={ADVANCED_SETTINGS.devMode.label}
                description={ADVANCED_SETTINGS.devMode.description}
                control={<ToggleControl checked={devModeOn()} onChange={setDevMode} />}
            />
        </>
    );
}
