// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * ProcessControls - Agent process management (pause, resume, kill, restart)
 */

import { useAtomValue } from "jotai";
import { PrimitiveAtom } from "jotai";
import clsx from "clsx";
import React, { memo, useCallback } from "react";
import type { AgentProcessState, StreamingState } from "../types";

interface ProcessControlsProps {
    processAtom: PrimitiveAtom<AgentProcessState>;
    streamingStateAtom: PrimitiveAtom<StreamingState>;
    onPause?: () => void;
    onResume?: () => void;
    onKill?: () => void;
    onRestart?: () => void;
}

export const ProcessControls: React.FC<ProcessControlsProps> = memo(
    ({ processAtom, streamingStateAtom, onPause, onResume, onKill, onRestart }) => {
        const processState = useAtomValue(processAtom);
        const streamingState = useAtomValue(streamingStateAtom);

        const handlePause = useCallback(() => {
            if (onPause) onPause();
        }, [onPause]);

        const handleResume = useCallback(() => {
            if (onResume) onResume();
        }, [onResume]);

        const handleKill = useCallback(() => {
            if (onKill && confirm("Are you sure you want to kill this agent process?")) {
                onKill();
            }
        }, [onKill]);

        const handleRestart = useCallback(() => {
            if (onRestart && confirm("Are you sure you want to restart this agent?")) {
                onRestart();
            }
        }, [onRestart]);

        const isStreaming = streamingState.active;
        const canPause = isStreaming && processState.status === "running";
        const canResume = !isStreaming && processState.status === "paused";
        const canKill = processState.canKill;
        const canRestart = processState.canRestart;

        return (
            <div className="agent-process-controls">
                <button
                    className={clsx("agent-control-btn", "pause-btn")}
                    disabled={!canPause}
                    onClick={handlePause}
                    title="Pause agent execution"
                >
                    ⏸ Pause
                </button>

                <button
                    className={clsx("agent-control-btn", "resume-btn")}
                    disabled={!canResume}
                    onClick={handleResume}
                    title="Resume agent execution"
                >
                    ▶ Resume
                </button>

                <button
                    className={clsx("agent-control-btn", "kill-btn")}
                    disabled={!canKill}
                    onClick={handleKill}
                    title="Kill agent process"
                >
                    ⏹ Kill
                </button>

                <button
                    className={clsx("agent-control-btn", "restart-btn")}
                    disabled={!canRestart}
                    onClick={handleRestart}
                    title="Restart agent"
                >
                    🔄 Restart
                </button>
            </div>
        );
    }
);

ProcessControls.displayName = "ProcessControls";
