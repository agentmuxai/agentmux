// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TextInput } from "@/app/element/ui";
import { type JSX } from "solid-js";

import { DEV_MODE_KEY, devModeOn, setDevMode } from "@/app/store/dev-mode";
import { settingsAtom } from "@/app/store/global";
import {
    KANBAN_REMINDERS_DESCRIPTION,
    KANBAN_REMINDERS_SETTING,
    kanbanRemindersOn,
} from "@/app/view/kanban/kanban-reminders";
import {
    resolveSkidNotches,
    SKID_NOTCHES_MAX,
    SKID_NOTCHES_SETTING,
} from "@/app/view/agent/components/scroll-handoff";
import type { SettingsIndexEntry } from "../settings-model";
import { KeyValueEditor, NumberControl, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";

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

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const ADVANCED_SETTINGS = {
    devMode: {
        id: "advanced.dev_mode",
        label: "Dev mode",
        description:
            "Shows DevTools in the menu, Inspect Element and View Page Source in right-click menus, and other developer tools. Off by default in the installed app; on by default in a task dev or debug build",
        section: "advanced",
        group: "Developer",
        keywords: ["developer", "devtools", "inspect element", "view source", "task dev", "debug build", DEV_MODE_KEY],
    },
    disableWebgl: {
        id: "advanced.disable_webgl",
        label: "Disable WebGL rendering",
        description: "Fall back to canvas-based terminal rendering (restart required)",
        section: "advanced",
        group: "Terminal (power user)",
        keywords: ["gpu rendering", "canvas fallback", "graphics glitch", "term:disablewebgl"],
    },
    autoAnswerTimeout: {
        id: "advanced.auto_answer_timeout",
        label: "Auto-answer timeout",
        description: "Seconds an AskUserQuestion panel waits for you before auto-selecting the recommended option(s)",
        section: "advanced",
        group: "Agent panes",
        keywords: ["auto select", "question timeout", "agent:askquestiontimeoutms"],
    },
    compactAt: {
        id: "advanced.compact_at",
        label: "Compact agents at",
        description:
            "Thousands of tokens at which Claude agents compact their context. Lower saves tokens; higher keeps more detail. 0 keeps Claude Code's own (about 967). Takes effect when each agent next starts",
        section: "advanced",
        group: "Agent panes",
        keywords: ["compaction", "context", "tokens", "autoCompactWindow", "agent:compactat"],
    },
    previewSkidNotches: {
        id: "advanced.preview_skid_notches",
        label: "Preview scroll skid",
        description:
            "Wheel notches a tool preview absorbs at its top or bottom edge before the scroll passes to the pane, so a preview sliding under the pointer is not scrolled past. 0 passes it on at once. Applies to open panes",
        section: "advanced",
        group: "Agent panes",
        keywords: ["scroll", "wheel", "notches", "tool preview", "edge", "agent:previewskidnotches"],
    },
    containerMemory: {
        id: "advanced.container_memory",
        label: "Container agents' memory",
        description:
            "Megabytes of memory each container agent may use, unless the agent has its own limit. auto is half of what Docker has (at least 2 GB). 0 is no limit. Reaches running containers without restarting them",
        section: "advanced",
        group: "Agent panes",
        keywords: ["container", "docker", "memory", "ram", "limit", "out of memory", "agent:container_memory"],
    },
    containerCpus: {
        id: "advanced.container_cpus",
        label: "Container agents' CPUs",
        description:
            "CPUs each container agent may use, unless the agent has its own limit. auto is all but a quarter of what Docker has (at least one). 0 is no limit. Reaches running containers without restarting them",
        section: "advanced",
        group: "Agent panes",
        keywords: ["container", "docker", "cpu", "cores", "limit", "agent:container_cpus"],
    },
    subagentModel: {
        id: "advanced.subagent_model",
        label: "Sub-agent model",
        description:
            "The model Claude agents' general-purpose sub-agents run on: sonnet, haiku, opus, a model ID, or inherit to use the agent's own. A sub-agent that names its own model, or one the agent passes per call, still wins. Takes effect when each agent next starts",
        section: "advanced",
        group: "Agent panes",
        keywords: ["sub-agent", "subagent", "model", "tokens", "agent:subagentmodel", "sonnet"],
    },
    exploreModel: {
        id: "advanced.explore_model",
        label: "Explore sub-agent model",
        description:
            "The model Claude agents' Explore sub-agents run on: haiku, sonnet, opus, a model ID, or inherit to leave Claude Code's built-in. Takes effect when each agent next starts",
        section: "advanced",
        group: "Agent panes",
        keywords: ["explore", "sub-agent", "subagent", "model", "tokens", "agent:exploremodel", "haiku"],
    },
    compactHints: {
        id: "advanced.compact_hints",
        label: "Suggest compacting",
        description:
            "When an agent's turn ends with a large context, tell it once that it can compact itself before its next task, so it doesn't compact by itself mid-task",
        section: "advanced",
        group: "Agent panes",
        keywords: ["compaction", "context", "tokens", "hint", "agent:compacthints"],
    },
    overlapNotes: {
        id: "advanced.overlap_notes",
        label: "Overlap notes",
        description:
            "Tell an agent when it edits a file another agent on this computer is also changing, naming that agent so it can message them first",
        section: "advanced",
        group: "Agent panes",
        keywords: ["same file", "conflict", "other agents", "Presence", "agent:overlapnotes"],
    },
    retryOnReconnect: {
        id: "advanced.retry_on_reconnect",
        label: "Retry after the API is back",
        description:
            "When a turn failed because the model API couldn't be reached, retry it once by itself when the API answers again, unless a tool ran in it or you typed in that pane since",
        section: "advanced",
        group: "Agent panes",
        keywords: ["offline", "network", "auto retry", "reconnect", "agent:retryonreconnect"],
    },
    kanbanReminders: {
        id: "advanced.kanban_reminders",
        label: "Remind idle agents of their backlog",
        description: KANBAN_REMINDERS_DESCRIPTION,
        section: "advanced",
        group: "Agent panes",
        keywords: ["kanban", "backlog", "reminder", "tasks waiting", "kanban:reminders"],
    },
    sampleInterval: {
        id: "advanced.sysinfo_sample_interval",
        label: "Sample interval",
        description: "Seconds between sysinfo widget samples",
        section: "advanced",
        group: "Sysinfo widget",
        keywords: ["sysinfo refresh rate", "polling interval", "telemetry:interval"],
    },
    historyLength: {
        id: "advanced.sysinfo_history_length",
        label: "History length",
        description: "Number of sysinfo widget samples retained (30–1024)",
        section: "advanced",
        group: "Sysinfo widget",
        keywords: ["sysinfo history", "graph length", "telemetry:numpoints"],
    },
    enableFileDrop: {
        id: "advanced.enable_file_drop",
        label: "Enable file drop",
        description: "Drag files onto an agent pane to attach them",
        section: "advanced",
        group: "Drag & drop",
        keywords: ["drag and drop", "file attach", "dnd:enabled"],
    },
    insertReferenceToken: {
        id: "advanced.insert_reference_token",
        label: "Insert reference token",
        description: "Also insert a file-reference token into the composer on drop, in addition to attaching the file",
        section: "advanced",
        group: "Drag & drop",
        keywords: ["file reference", "drop token", "dnd:agentinserttoken"],
    },
    maxConcurrentUploads: {
        id: "advanced.max_concurrent_uploads",
        label: "Max concurrent uploads",
        description: "Files uploaded at once on a multi-file drop. Leave blank for unlimited.",
        section: "advanced",
        group: "Drag & drop",
        keywords: ["upload concurrency", "parallel uploads", "dnd:concurrency"],
    },
    globalEnvVars: {
        id: "advanced.global_env_vars",
        label: "Global environment variables",
        description: "Environment variables injected into every shell",
        section: "advanced",
        group: "Environment",
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
            <SectionHeader label="Developer" />
            <SettingRow
                id={ADVANCED_SETTINGS.devMode.id}
                label={ADVANCED_SETTINGS.devMode.label}
                description={ADVANCED_SETTINGS.devMode.description}
                control={<ToggleControl checked={devModeOn()} onChange={setDevMode} />}
            />
        </div>
    );
}
