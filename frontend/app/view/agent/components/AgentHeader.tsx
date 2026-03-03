// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentHeader - Agent status display and controls
 */

import { useAtomValue } from "jotai";
import { PrimitiveAtom } from "jotai";
import clsx from "clsx";
import React, { memo } from "react";
import type { AgentProcessState, MessageRouterState, StreamingState } from "../types";
import { ProcessControls } from "./ProcessControls";

interface AgentHeaderProps {
    agentId: string;
    processAtom: PrimitiveAtom<AgentProcessState>;
    streamingStateAtom: PrimitiveAtom<StreamingState>;
    messageRouterAtom: PrimitiveAtom<MessageRouterState>;
    onPause?: () => void;
    onResume?: () => void;
    onKill?: () => void;
    onRestart?: () => void;
}

export const AgentHeader: React.FC<AgentHeaderProps> = memo(
    ({ agentId, processAtom, streamingStateAtom, messageRouterAtom, onPause, onResume, onKill, onRestart }) => {
        const processState = useAtomValue(processAtom);
        const streamingState = useAtomValue(streamingStateAtom);
        const routerState = useAtomValue(messageRouterAtom);

        const statusIcon = {
            idle: "⚪",
            running: "🟢",
            paused: "🟡",
            failed: "🔴",
        }[processState.status];

        const backendIcon = routerState.backend === "local" ? "🖥️" : "☁️";

        return (
            <div className="agent-header">
                <div className="agent-header-info">
                    <div className="agent-header-title">
                        <span className="agent-status-icon">{statusIcon}</span>
                        <span className="agent-id">Agent: {agentId}</span>
                        <span className="agent-backend-icon" title={`Backend: ${routerState.backend}`}>
                            {backendIcon}
                        </span>
                    </div>
                    <div className="agent-header-stats">
                        {processState.pid && <span className="agent-pid">PID: {processState.pid}</span>}
                        <span
                            className={clsx("agent-connection-status", {
                                connected: routerState.connected,
                                disconnected: !routerState.connected,
                            })}
                        >
                            {routerState.connected ? "Connected" : "Disconnected"}
                        </span>
                        {streamingState.active && (
                            <span className="agent-streaming-indicator">
                                ⏳ Streaming ({streamingState.bufferSize} events)
                            </span>
                        )}
                    </div>
                </div>

                <ProcessControls
                    processAtom={processAtom}
                    streamingStateAtom={streamingStateAtom}
                    onPause={onPause}
                    onResume={onResume}
                    onKill={onKill}
                    onRestart={onRestart}
                />
            </div>
        );
    }
);

AgentHeader.displayName = "AgentHeader";
