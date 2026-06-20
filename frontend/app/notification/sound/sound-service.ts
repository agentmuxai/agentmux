// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sound service — the orchestrator.
 *
 * Spec: docs/specs/SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §4.6.
 *
 * Responsibilities:
 *   1. Prime the AudioContext on the first user gesture (autoplay
 *      policy).
 *   2. Hold the SoundPlayer and route every sound event through it.
 *   3. Track which agent-pane events translate to which sound IDs
 *      (subscribed via the agent-pane-state-store multicast).
 *   4. Apply settings gating (master enable, per-event enable,
 *      master volume), coalesce, and focus suppression.
 *   5. Honor replay mode so historical events don't play sound.
 *
 * Installed once from app-init via `installSoundService()`.
 */

import { focusManager } from "@/app/store/focusManager";
import { getSettingsKeyAtom } from "@/app/store/global";
import { makeWindowFocusSignal } from "@/app/window/window-focus";
import {
    addEventListener as addPaneListener,
    type AgentPaneEvent,
} from "@/app/store/agent-pane-state-store";
import { createEffect, createRoot } from "solid-js";
import { notify, subscribeSoundEvents, type SoundEvent } from "./sound-events";
import { SOUNDS, type SoundId } from "./sounds";
import { SoundPlayer } from "./sound-player";
import { ToolTonesPlayer } from "./tool-tones-player";
import { WaitingTonePlayer } from "./waiting-tone-player";

let installed = false;
let replayMode = false;
const player = new SoundPlayer();
const toolTones = new ToolTonesPlayer();
const waitingTones = new Map<string, WaitingTonePlayer>(); // blockId → player
const waitingTimeouts = new Map<string, ReturnType<typeof setTimeout>>();
const lastFiredAt = new Map<SoundId, number>();

const WAITING_AUTO_STOP_MS = 5 * 60 * 1000;

/**
 * Toggle replay mode. While true, the bus still receives events but
 * the service drops every play. The session-replay infrastructure
 * (SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md) flips this around
 * its historical dispatches so the user doesn't hear a din of past
 * turn-completions when scrubbing a replay.
 */
export function setReplayMode(value: boolean): void {
    replayMode = value;
}

export function isReplayMode(): boolean {
    return replayMode;
}

/**
 * Install the sound service. Idempotent — subsequent calls no-op.
 * Returns a cleanup function for tests; production code does not
 * uninstall.
 */
export function installSoundService(): () => void {
    if (installed) return () => undefined;
    installed = true;

    const windowFocused = makeWindowFocusSignal();

    // Prime AudioContext on the first user gesture, exactly once.
    // After priming, hook the tool-tones chain into the same context +
    // master gain so a single OS volume slider controls both subsystems.
    const onceOpts: AddEventListenerOptions = { capture: true, once: true };
    const primeOnce = async () => {
        document.removeEventListener("pointerdown", primeOnce, true);
        document.removeEventListener("keydown", primeOnce, true);
        await player.prime();
        const ctx = player.getAudioContext();
        const master = player.getMasterGain();
        if (ctx && master) {
            if (!toolTones.isAttached()) toolTones.attach(ctx, master);
            // Attach any waiting players that were created before prime.
            for (const wp of waitingTones.values()) {
                if (!wp.isAttached()) wp.attach(ctx, master);
            }
        }
    };
    document.addEventListener("pointerdown", primeOnce, onceOpts);
    document.addEventListener("keydown", primeOnce, onceOpts);

    // Reactively thread the master-volume + tool-tones-volume settings
    // into their respective gain nodes.
    const volumeDispose = createRoot((dispose) => {
        createEffect(() => {
            const vol = getSettingsKeyAtom("notify:sounds:volume")();
            player.setMasterGain(typeof vol === "number" ? vol : 0.6);
        });
        createEffect(() => {
            const vol = getSettingsKeyAtom("notify:tooltones:volume")();
            toolTones.setVolume(typeof vol === "number" ? vol : 0.15);
        });
        createEffect(() => {
            const vol = getSettingsKeyAtom("notify:sounds:waiting:volume")();
            const v = typeof vol === "number" ? vol : 0.25;
            for (const wp of waitingTones.values()) wp.setVolume(v);
        });
        return dispose;
    });

    // Bus → player.
    const busUnsub = subscribeSoundEvents((ev) => {
        if (replayMode) return;
        if (!shouldPlay(ev, windowFocused)) return;
        const def = SOUNDS[ev.id];
        if (!def) return;
        const now =
            typeof performance !== "undefined" && performance.now
                ? performance.now()
                : Date.now();
        const last = lastFiredAt.get(ev.id) ?? 0;
        if (now - last < (def.coalesceMs ?? 300)) return;
        lastFiredAt.set(ev.id, now);
        try {
            player.play(def, ev.override?.gain ?? 1);
        } catch (e) {
            console.warn(`[sound] play threw for ${ev.id}`, e);
        }
    });

    // Reducer events → bus.
    const paneUnsub = addPaneListener((blockId, event) => {
        mapAgentPaneEvent(blockId, event);
    });

    return () => {
        busUnsub();
        paneUnsub();
        volumeDispose();
        document.removeEventListener("pointerdown", primeOnce, true);
        document.removeEventListener("keydown", primeOnce, true);
        installed = false;
    };
}

function mapAgentPaneEvent(blockId: string, event: AgentPaneEvent): void {
    switch (event.type) {
        case "tool-started":
            playToolToneIfAllowed(blockId, event.name);
            return;
        case "turn-ended":
            if (event.outcome === "completed") {
                notify("agent.turn.complete", { sourceBlockId: blockId });
            } else if (event.outcome === "errored") {
                notify("agent.turn.error", { sourceBlockId: blockId });
            } else if (
                event.outcome === "stopped" ||
                event.outcome === "interrupted"
            ) {
                notify("agent.turn.interrupted", { sourceBlockId: blockId });
            }
            return;
        case "submit-timed-out":
            notify("agent.turn.error", { sourceBlockId: blockId });
            return;
        case "interrupt-timed-out":
            notify("agent.turn.interrupted", { sourceBlockId: blockId });
            return;
        case "pending-accepted":
            if (event.wasPresent) {
                notify("agent.message.accepted", { sourceBlockId: blockId });
            }
            return;
        case "pending-rejected":
            if (event.wasPresent) {
                notify("agent.message.rejected", { sourceBlockId: blockId });
            }
            return;
        case "waiting-for-input":
            startWaiting(blockId);
            return;
        case "waiting-ended":
            stopWaiting(blockId);
            return;
    }
}

function startWaiting(blockId: string): void {
    if (replayMode) return;
    if (getSettingsKeyAtom("notify:sounds:enabled")() === false) return;
    if (getSettingsKeyAtom("notify:sound:agent.waiting.for.input")() === false) return;

    let wp = waitingTones.get(blockId);
    if (!wp) {
        wp = new WaitingTonePlayer();
        const ctx = player.getAudioContext();
        const master = player.getMasterGain();
        if (ctx && master) wp.attach(ctx, master);
        waitingTones.set(blockId, wp);
    }

    const vol =
        (getSettingsKeyAtom("notify:sounds:waiting:volume")() as number | undefined) ?? 0.25;
    wp.start(vol);

    // 5-minute safety cutoff
    const existing = waitingTimeouts.get(blockId);
    if (existing !== undefined) clearTimeout(existing);
    waitingTimeouts.set(
        blockId,
        setTimeout(() => stopWaiting(blockId), WAITING_AUTO_STOP_MS),
    );
}

function stopWaiting(blockId: string): void {
    const t = waitingTimeouts.get(blockId);
    if (t !== undefined) {
        clearTimeout(t);
        waitingTimeouts.delete(blockId);
    }
    waitingTones.get(blockId)?.stop();
}

/**
 * Tool-tone playback path. Separate from the SoundEvent bus because
 * the policy is different (on-by-default, scope-based instead of
 * focus-suppressing) and the player is a different chain.
 *
 * Spec: docs/specs/SPEC_AGENT_TOOL_CALL_TONES_2026_06_05.md §6.
 */
function playToolToneIfAllowed(blockId: string, tool: string): void {
    if (replayMode) return;
    // v1 master kill-switch silences tool tones too (shared chain).
    if (getSettingsKeyAtom("notify:sounds:enabled")() === false) return;
    // Tool-tones enable; absence = default on.
    if (getSettingsKeyAtom("notify:tooltones:enabled")() === false) return;
    const scope = getSettingsKeyAtom("notify:tooltones:scope")() ?? "all";
    if (scope === "focused") {
        const windowFocused = makeWindowFocusSignal();
        if (
            focusManager.blockFocusAtom() !== blockId ||
            !windowFocused()
        ) {
            return;
        }
    }
    // "window" mode (v1.5) falls through to "all" for now; see spec §8.5.
    const ctx = player.getAudioContext();
    if (!ctx || !toolTones.isAttached()) return; // not primed yet
    try {
        toolTones.play(ctx, tool);
    } catch (e) {
        console.warn(`[sound] tool-tone play threw for ${tool}`, e);
    }
}

function shouldPlay(ev: SoundEvent, windowFocused: () => boolean): boolean {
    const master = getSettingsKeyAtom("notify:sounds:enabled")();
    if (master === false) return false;
    const def = SOUNDS[ev.id];
    if (!def) return false;
    const perEvent = getSettingsKeyAtom(def.settingKey)();
    if (perEvent === false) return false;

    const suppressRaw = getSettingsKeyAtom(
        "notify:sounds:suppresswhenfocused",
    )();
    const suppressWhenFocused = suppressRaw !== false; // default true
    if (
        suppressWhenFocused &&
        ev.sourceBlockId &&
        focusManager.blockFocusAtom() === ev.sourceBlockId &&
        windowFocused()
    ) {
        return false;
    }
    return true;
}

// ── Test helpers (NEVER call from production) ─────────────────────────

export function __resetSoundService(): void {
    installed = false;
    replayMode = false;
    lastFiredAt.clear();
    toolTones.__resetCoalesce();
    for (const t of waitingTimeouts.values()) clearTimeout(t);
    waitingTimeouts.clear();
    waitingTones.clear();
}

export function __getWaitingTones(): Map<string, WaitingTonePlayer> {
    return waitingTones;
}

export function __getSoundPlayer(): SoundPlayer {
    return player;
}

export function __getToolTonesPlayer(): ToolTonesPlayer {
    return toolTones;
}
