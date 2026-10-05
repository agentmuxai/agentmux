// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// srv's list of remotes (`remoteslist`), shared by the connection picker and
// Hangar's sidebar (SPEC_REMOTES_PANE_2026_10_05.md §4.7), so they show what
// the Remotes pane shows. Fetched on first use, then again whenever a
// connection, the list or the settings change.

import { createSignal, type Accessor } from "solid-js";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { TabRpcClient } from "@/app/store/rpc-util";

const [records, setRecords] = createSignal<RemoteRecord[]>([]);
let started = false;
let timer: ReturnType<typeof setTimeout> | null = null;

/** Fetch the list now. A failure keeps the last one. */
export async function refreshRemotes(): Promise<void> {
    try {
        setRecords((await RpcApi.RemotesListCommand(TabRpcClient, { timeout: 5000 })) ?? []);
    } catch (e) {
        console.log("remotes: could not load the list", e);
    }
}

function scheduleRefresh(): void {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
        timer = null;
        void refreshRemotes();
    }, 150);
}

/** The list, kept current from the first call on. */
export function remotesList(): Accessor<RemoteRecord[]> {
    if (!started) {
        started = true;
        muxEventSubscribe(
            { eventType: WpsEvent.ConnChange, handler: scheduleRefresh },
            { eventType: WpsEvent.RemotesChange, handler: scheduleRefresh },
            { eventType: WpsEvent.Config, handler: scheduleRefresh }
        );
        void refreshRemotes();
    }
    return records;
}
