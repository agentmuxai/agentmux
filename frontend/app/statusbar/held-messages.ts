// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The messages srv holds for agents, as counts (`jekt.held.summary`, srv's `server/jekt_held_summary.rs`): one shared copy,
// read while the status bar's summary is mounted. It reads again on the `jektheld` event and when an agent's state changes
// (a held message's reason follows its agent's state), so nothing polls.

import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import type { JektHeldAgent } from "@/types/rpc/JektHeldAgent";

const DEBOUNCE_MS = 300;

const [held, setHeld] = createSignal<JektHeldAgent[]>([]);
let users = 0;
let unsub: (() => void) | null = null;
let debounce: ReturnType<typeof setTimeout> | null = null;

async function refresh(): Promise<void> {
    try {
        const r = await RpcApi.JektHeldSummaryCommand(TabRpcClient);
        setHeld(r?.agents ?? []);
    } catch {
        // An older srv, or none yet: no part.
    }
}

const refreshSoon = (): void => {
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => void refresh(), DEBOUNCE_MS);
};

/** The agents that have messages held for them, kept current while the calling component is mounted. */
export function useHeldMessages(): Accessor<JektHeldAgent[]> {
    if (users++ === 0) {
        void refresh();
        unsub = onMuxEvent(WpsEvent.JektHeld, refreshSoon);
    }
    // An agent stopping, signing out or running again changes why its messages are held.
    let seen = "";
    createEffect(() => {
        const sig = sharedAgentRuntime()
            .rows()
            .map((r) => `${r.agent_id}:${r.state}`)
            .join(",");
        if (seen !== "" && sig !== seen) refreshSoon();
        seen = sig;
    });
    onCleanup(() => {
        if (--users > 0) return;
        unsub?.();
        unsub = null;
        if (debounce) clearTimeout(debounce);
        debounce = null;
    });
    return held;
}
