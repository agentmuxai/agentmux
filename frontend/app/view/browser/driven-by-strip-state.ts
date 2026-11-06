// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The driven-by strip's state (driven-by-strip.tsx): turning Auto-approve on (its confirmation, the duration, the
 * grant) and off, Pause and Resume, and the Log. Each failure is worded for the person and shown where it happened.
 */

import { browserPauseAll, setAgentBrowsingPaused } from "@/app/store/browser-pause-all";
import { createSignal } from "solid-js";
import type { BrowserViewModel } from "./browser-model";
import { plainError } from "./plain-error";

export interface DrivenByProps {
    model: BrowserViewModel;
    agent: string;
}

/** Auto-approve: the confirmation, the duration picked, the grant, and stopping it. */
function createBypassControl(model: BrowserViewModel) {
    const [confirming, setConfirming] = createSignal(false);
    const [minutes, setMinutes] = createSignal(60);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | undefined>();
    const sites = () => model.bypassOriginsPreview();

    const turnOn = (e: MouseEvent) => {
        // Only a pointer click grants it: a key press on the focused button
        // (detail 0) doesn't, so no synthesized key can (spec §3.2).
        if (e.detail === 0 || busy()) return;
        setBusy(true);
        setError(undefined);
        model
            .setBypass(true, minutes())
            .then(() => setConfirming(false))
            .catch((err) => setError(plainError(err, "turn on auto-approve")))
            .finally(() => setBusy(false));
    };
    // A failed Stop auto-approve shows in the red strip itself: the
    // confirmation, where other errors show, is hidden while it is on.
    const end = () => {
        setError(undefined);
        model.setBypass(false).catch((err) => setError(plainError(err, "stop auto-approve", (m) => `Couldn't stop auto-approve: ${m}`)));
    };
    return { confirming, setConfirming, minutes, setMinutes, busy, error, setError, sites, turnOn, end };
}

/** Pause and Resume, which also resume every pane when "Pause all agent browsing" is what holds this one. */
function createPauseControl(model: BrowserViewModel) {
    const [pauseError, setPauseError] = createSignal<string | undefined>();
    // Pause is the person's brake, so any press works; Resume lets the agent
    // act again, so only a pointer click does, like turning Auto-approve on.
    // "Pause all agent browsing" holds this pane too: its Resume turns that off, for every pane.
    const everyPane = () => browserPauseAll().paused;
    const togglePause = (e: MouseEvent) => {
        const paused = model.pausedAtom();
        if (paused && e.detail === 0) return;
        setPauseError(undefined);
        (paused && everyPane() ? setAgentBrowsingPaused(false) : model.setPaused(!paused))
            .catch((err) => {
                const what = paused ? "resume the agent" : "pause the agent";
                setPauseError(plainError(err, what, (m) => `Couldn't ${paused ? "resume" : "pause"}: ${m}`));
            });
    };
    return { pauseError, everyPane, togglePause };
}

/** The strip's state: everything its markup reads, and what its controls do. */
export function createDrivenBy(props: DrivenByProps) {
    const model = props.model;
    const bypass = createBypassControl(model);
    const [showLog, setShowLog] = createSignal(false);
    const pause = createPauseControl(model);
    return { model, ...bypass, ...pause, showLog, setShowLog };
}

/** What the driven-by strip's markup reads. */
export type DrivenByState = ReturnType<typeof createDrivenBy>;
