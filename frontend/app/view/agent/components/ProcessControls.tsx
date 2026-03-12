// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * ProcessControls - Agent process management (pause, resume, kill, restart)
 */

import clsx from "clsx";
import { type JSX } from "solid-js";
import type { SignalPair } from "../state";
import type { AgentProcessState, StreamingState } from "../types";

interface ProcessControlsProps {
    processAtom: SignalPair<AgentProcessState>;
    streamingStateAtom: SignalPair<StreamingState>;
    onPause?: () => void;
    onResume?: () => void;
    onKill?: () => void;
    onRestart?: () => void;
}

export const ProcessControls = ({
    processAtom,
    streamingStateAtom,
    onPause,
    onResume,
    onKill,
    onRestart,
}: ProcessControlsProps): JSX.Element => {
    const [processState] = processAtom;
    const [streamingState] = streamingStateAtom;

    const handlePause = () => {
        if (onPause) onPause();
    };

    const handleResume = () => {
        if (onResume) onResume();
    };

    const handleKill = () => {
        if (onKill && confirm("Are you sure you want to kill this agent process?")) {
            onKill();
        }
    };

    const handleRestart = () => {
        if (onRestart && confirm("Are you sure you want to restart this agent?")) {
            onRestart();
        }
    };

    const isStreaming = () => streamingState().active;
    const canPause = () => isStreaming() && processState().status === "running";
    const canResume = () => !isStreaming() && processState().status === "paused";
    const canKill = () => processState().canKill;
    const canRestart = () => processState().canRestart;

    return (
        <div class="agent-process-controls">
            <button
                class={clsx("agent-control-btn", "pause-btn")}
                disabled={!canPause()}
                onClick={handlePause}
                title="Pause agent execution"
            >
                ⏸ Pause
            </button>

            <button
                class={clsx("agent-control-btn", "resume-btn")}
                disabled={!canResume()}
                onClick={handleResume}
                title="Resume agent execution"
            >
                ▶ Resume
            </button>

            <button
                class={clsx("agent-control-btn", "kill-btn")}
                disabled={!canKill()}
                onClick={handleKill}
                title="Kill agent process"
            >
                ⏹ Kill
            </button>

            <button
                class={clsx("agent-control-btn", "restart-btn")}
                disabled={!canRestart()}
                onClick={handleRestart}
                title="Restart agent"
            >
                🔄 Restart
            </button>
        </div>
    );
};

ProcessControls.displayName = "ProcessControls";
