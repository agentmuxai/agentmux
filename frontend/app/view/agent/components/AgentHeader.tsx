// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentHeader - Agent status display and controls
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import type { SignalPair } from "../state";
import type { AgentProcessState, MessageRouterState, StreamingState } from "../types";
import { ProcessControls } from "./ProcessControls";

interface AgentHeaderProps {
    agentId: string;
    processAtom: SignalPair<AgentProcessState>;
    streamingStateAtom: SignalPair<StreamingState>;
    messageRouterAtom: SignalPair<MessageRouterState>;
    onPause?: () => void;
    onResume?: () => void;
    onKill?: () => void;
    onRestart?: () => void;
}

export const AgentHeader = ({
    agentId,
    processAtom,
    streamingStateAtom,
    messageRouterAtom,
    onPause,
    onResume,
    onKill,
    onRestart,
}: AgentHeaderProps): JSX.Element => {
    const [processState] = processAtom;
    const [streamingState] = streamingStateAtom;
    const [routerState] = messageRouterAtom;

    const statusIcon = () => ({
        idle: "⚪",
        running: "🟢",
        paused: "🟡",
        failed: "🔴",
    }[processState().status]);

    const backendIcon = () => routerState().backend === "local" ? "🖥️" : "☁️";

    return (
        <div class="agent-header">
            <div class="agent-header-info">
                <div class="agent-header-title">
                    <span class="agent-status-icon">{statusIcon()}</span>
                    <span class="agent-id">Agent: {agentId}</span>
                    <span class="agent-backend-icon" title={`Backend: ${routerState().backend}`}>
                        {backendIcon()}
                    </span>
                </div>
                <div class="agent-header-stats">
                    <Show when={processState().pid}>
                        <span class="agent-pid">PID: {processState().pid}</span>
                    </Show>
                    <span
                        class={clsx("agent-connection-status", {
                            connected: routerState().connected,
                            disconnected: !routerState().connected,
                        })}
                    >
                        {routerState().connected ? "Connected" : "Disconnected"}
                    </span>
                    <Show when={streamingState().active}>
                        <span class="agent-streaming-indicator">
                            ⏳ Streaming ({streamingState().bufferSize} events)
                        </span>
                    </Show>
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
};

AgentHeader.displayName = "AgentHeader";
