// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the store and the agent pane share about an open that srv held for a shared folder (`agent.open` refused a host
// agent whose folder a container agent can write until the person has gone ahead; view/agent/shared-folder-ask.ts).

/** The block meta key of an agent pane that is to open an agent (a layout's placeholder, a pane the window made to ask in). */
export const OPEN_ASK_META = "layout:agent";

/** Whether srv refused an `agent.open` because of a shared folder: its refusal starts with this word. */
export function isSharedFolderRefusal(message: string): boolean {
    return message.includes("SHARED_FOLDER: ");
}
