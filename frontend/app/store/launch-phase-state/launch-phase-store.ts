// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Solid binding for the launch-phase-state reducer, one per agent pane
 * (created by `useAgentControllerStatus`, so no slot map). Each field is a
 * memo, so a consumer re-runs only when its own field changes: the same
 * granularity as the separate signals this replaced. Every dispatch goes to
 * the global audit ring.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";

import { type CommandSource, recordDispatch } from "../command-source";
import { update } from "./reducer";
import {
    initialState,
    type AuthStatus,
    type LaunchPhaseCommand,
    type LaunchPhaseEvent,
    type LaunchPhaseState,
} from "./types";
import type { LaunchPhase } from "@/app/view/agent/flows/launch-phase";

export interface LaunchPhaseStore {
    /** Runs one command; returns the events it produced. */
    dispatch: (cmd: LaunchPhaseCommand, source?: CommandSource) => LaunchPhaseEvent[];
    /** Current state, untracked. For decisions inside async flows, not rendering. */
    snapshot: () => LaunchPhaseState;
    phase: Accessor<LaunchPhase | null>;
    mountRunning: Accessor<boolean>;
    agentReady: Accessor<boolean>;
    canRetry: Accessor<boolean>;
    authStatus: Accessor<AuthStatus>;
    authUrl: Accessor<string | null>;
    authNotice: Accessor<string | null>;
    loginWaiting: Accessor<boolean>;
}

/** Must run under a Solid owner (the pane's component scope). */
export function createLaunchPhaseStore(key: string, nowMs: () => number = Date.now): LaunchPhaseStore {
    let current = initialState();
    const [state, setState] = createSignal<LaunchPhaseState>(current);
    const dispatch = (cmd: LaunchPhaseCommand, source: CommandSource = "system"): LaunchPhaseEvent[] => {
        const result = update(current, cmd, nowMs());
        if (result.state !== current) {
            current = result.state;
            setState(current);
        }
        recordDispatch({ slice: "launch-phase-state", key, command: cmd, events: result.events, source, at: Date.now() });
        return result.events;
    };
    return {
        dispatch,
        snapshot: () => current,
        phase: createMemo(() => state().phase),
        mountRunning: createMemo(() => state().mountRunning),
        agentReady: createMemo(() => state().agentReady),
        canRetry: createMemo(() => state().canRetry),
        authStatus: createMemo(() => state().authStatus),
        authUrl: createMemo(() => state().authUrl),
        authNotice: createMemo(() => state().authNotice),
        loginWaiting: createMemo(() => state().recoveryFlows > 0),
    };
}
