// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { SettingsIndexEntry } from "../settings-model";

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
        group: "Tool-call tones",
        keywords: ["tool call tones", "tool call sound", "synth tone", "notify:tooltones:enabled"],
    },
    toolTonesVolume: {
        id: "sounds.tool_tones_volume",
        label: "Volume",
        section: "sounds",
        group: "Tool-call tones",
        keywords: ["tool tone volume", "notify:tooltones:volume"],
    },
    toolTonesScope: {
        id: "sounds.tool_tones_scope",
        label: "Scope",
        description: "Which panes play tool-call tones",
        section: "sounds",
        group: "Tool-call tones",
        keywords: ["tool tone scope", "focused only", "all panes", "notify:tooltones:scope"],
    },
    toolTonesFlash: {
        id: "sounds.tool_tones_flash",
        label: "Flash the tab and pane when a sound plays",
        description: "Pulse the pane tab each sound came from, in that pane's color, and subtly its window tab — one pulse per knock, in time with the sound",
        section: "sounds",
        keywords: ["activity flash", "tab flash", "flash with sounds", "highlight tab", "which pane", "notify:tooltones:flash"],
    },
    waitingToneEnabled: {
        id: "sounds.waiting_tone_enabled",
        label: "Enable",
        description:
            "Play a looping tone while an agent waits for you: a question, a tool permission, a browser hand-off or approval, an SSH consent or a widget install",
        section: "sounds",
        group: "Waiting for input",
        keywords: ["waiting for input sound", "ambient tone", "blocked sound", "permission", "approval", "notify:sound:agent.waiting.for.input"],
    },
    waitingBrowser: {
        id: "sounds.waiting_browser",
        label: "Browser hand-offs and approvals",
        description: "Also when an agent hands you a browser pane or asks to approve something in it",
        section: "sounds",
        group: "Waiting for input",
        keywords: ["browser", "hand-off", "handoff", "approval", "notify:waiting:browser"],
    },
    shutdownToneEnabled: {
        id: "sounds.shutdown_tone_enabled",
        label: "Shutdown warning chime",
        description: "Play a falling chime when something other than you asks to shut an agent down, at the start of the window to keep it and again with 5 seconds left",
        section: "sounds",
        group: "Shutdown warning",
        keywords: ["shutdown sound", "quit warning", "keep running", "notify:sound:agent.shutdown.pending"],
    },
    waitingToneVolume: {
        id: "sounds.waiting_tone_volume",
        label: "Volume",
        section: "sounds",
        group: "Waiting for input",
        keywords: ["waiting tone volume", "notify:sounds:waiting:volume"],
    },
} satisfies Record<string, SettingsIndexEntry>;
