// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The sessions a History row shows when it is opened, and keeping them current as the list moves.

import { createEffect, createSignal, untrack, type Accessor } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { SESSIONS_TIMEOUT_MS } from "@/app/view/agent/history/HistoryNavigator";
import type { HistoryAgent } from "@/types/rpc/HistoryAgent";
import type { HistoryAgentSessions } from "@/types/rpc/HistoryAgentSessions";

export type Sessions = { loading: true } | { loading: false; data?: HistoryAgentSessions; error?: string };

/**
 * The open rows' sessions, by agent id. A row's sessions are read when it is opened, and read again
 * when the list shows the agent gained an archived conversation. A growing transcript is not one
 * (reading the record is the slow part), so the rest wait for the next open or Refresh.
 */
export function useOpenSessions(agents: Accessor<HistoryAgent[] | null>) {
    const [open, setOpen] = createSignal<Record<string, Sessions>>({});

    const loadSessions = async (id: string) => {
        // What is shown stays until the new answer arrives, so a row doesn't blink.
        if (!open()[id]) setOpen({ ...open(), [id]: { loading: true } });
        try {
            const data = await RpcApi.HistoryAgentSessionsCommand(TabRpcClient, { definition_id: id }, { timeout: SESSIONS_TIMEOUT_MS });
            if (open()[id]) setOpen({ ...open(), [id]: { loading: false, data } });
        } catch (e) {
            if (open()[id]) setOpen({ ...open(), [id]: { loading: false, error: String(e) } });
        }
    };

    const toggleSessions = async (id: string) => {
        if (open()[id]) {
            const { [id]: _, ...rest } = open();
            setOpen(rest);
            return;
        }
        await loadSessions(id);
    };

    const archivesSeen = new Map<string, number>();
    createEffect(() => {
        for (const a of agents() ?? []) {
            const was = archivesSeen.get(a.definition_id);
            archivesSeen.set(a.definition_id, a.archives);
            if (was !== undefined && was !== a.archives && untrack(() => open()[a.definition_id])) untrack(() => void loadSessions(a.definition_id));
        }
    });

    return { open, closeAll: () => setOpen({}), toggleSessions };
}
