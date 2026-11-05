// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentDefinition } from "@/app/store/rpc-api";
import type { JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import "./agent-view.scss";
import { registerAgentViewCommands } from "./agent-view-commands";
import { useFailureHandling } from "./agent-view-failure";
import { usePaneFoundation } from "./agent-view-foundation";
import { usePaneInput } from "./agent-view-input";
import type { PaneLate } from "./agent-view-late";
import { AgentViewLayout } from "./agent-view-layout";
import { useSending } from "./agent-view-sending";
import { useTurnStreams } from "./agent-view-streams";
import { useHistoryAndFeed, useShownDocument } from "./agent-view-transcript";
import { useTurnAndLaunch } from "./agent-view-turn";

// The palette's "Agent view" commands exist from the first agent pane on; they are available only while one is focused.
registerAgentViewCommands();

/**
 * One agent's pane. The setup is a run of hook groups, each in its own file (agent-view-*.ts), called in
 * the order their hooks must be created; `late` carries what an early hook's callback reads from a later
 * group. agent-view-layout.tsx draws what they built.
 */
export const AgentPresentationView = ({
    model,
    agentId,
    agentDefinitions,
    progressBarMount,
}: {
    model: AgentViewModel;
    agentId: string;
    /** The wrapper's definition list: a second `useAgentDefinitions()` here would repeat its RPC and subscription. */
    agentDefinitions: () => AgentDefinition[];
    /** Where the progress bar portals: a node AgentPaneChrome owns, bridged through the model (`progressBarMount` in agent-model.ts). */
    progressBarMount: () => HTMLDivElement | undefined;
}): JSX.Element => {
    const late = { scrollToBottom: null, composerIsEmpty: null } as PaneLate;
    const found = usePaneFoundation(model, agentId, agentDefinitions);
    const feed = useHistoryAndFeed(found, model, agentId);
    const shown = useShownDocument(found, model, agentId, feed);
    const turn = useTurnAndLaunch({ found, model, agentId, agentDefinitions, feed, shown, late });
    const streams = useTurnStreams({ found, model, feed, turn, late });
    const sending = useSending({ found, model, turn, streams, late });
    late.commands = sending.commands;
    const failure = useFailureHandling({ found, model, agentId, turn, sending, shown, late });
    const input = usePaneInput({ found, model, shown, late });

    return <AgentViewLayout v={{ model, agentId, progressBarMount, found, feed, shown, turn, streams, sending, failure, input, late }} />;
};

AgentPresentationView.displayName = "AgentPresentationView";
