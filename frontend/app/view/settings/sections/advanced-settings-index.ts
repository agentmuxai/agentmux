// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { DEV_MODE_KEY } from "@/app/store/dev-mode";
import { KANBAN_REMINDERS_DESCRIPTION } from "@/app/view/kanban/kanban-reminders";
import type { SettingsIndexEntry } from "../settings-model";

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
