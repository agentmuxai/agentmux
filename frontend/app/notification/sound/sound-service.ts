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

let installed = false;
let replayMode = false;
const player = new SoundPlayer();
const lastFiredAt = new Map<SoundId, number>();

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
    const onceOpts: AddEventListenerOptions = { capture: true, once: true };
    const primeOnce = () => {
        void player.prime();
        document.removeEventListener("pointerdown", primeOnce, true);
        document.removeEventListener("keydown", primeOnce, true);
    };
    document.addEventListener("pointerdown", primeOnce, onceOpts);
    document.addEventListener("keydown", primeOnce, onceOpts);

    // Reactively thread the master-volume setting into the player.
    const volumeDispose = createRoot((dispose) => {
        createEffect(() => {
            const vol = getSettingsKeyAtom("notify:sounds:volume")();
            player.setMasterGain(typeof vol === "number" ? vol : 0.6);
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
        case "interrupt-timed-out":
            notify("agent.turn.error", { sourceBlockId: blockId });
            return;
        case "stream-stalled":
            notify("agent.stream.stalled", { sourceBlockId: blockId });
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
}

export function __getSoundPlayer(): SoundPlayer {
    return player;
}
