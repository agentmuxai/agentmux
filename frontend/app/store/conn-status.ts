// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Connection status — split out of global.ts (see global.ts's "Connection
// status" section for the original context). Re-exported from global.ts for
// backward-compat (97 files import from that module).

import { createMemo, createSignal } from "solid-js";
import { isBlank } from "@/util/util";
import { WpsEvent } from "@/app/store/wps-events";
import { waveEventSubscribe } from "./wps";
import { ClientService } from "./services";

// Connection status map: connName → ConnStatus signal
const [connStatusMap, setConnStatusMap] = createSignal(
    new Map<string, [() => ConnStatus, (v: ConnStatus) => void]>()
);

export const allConnStatus = createMemo<ConnStatus[]>(() => {
    const map = connStatusMap();
    return Array.from(map.values()).map(([get]) => get());
});

export async function loadConnStatus() {
    const connStatusArr = await ClientService.GetAllConnStatus();
    if (connStatusArr == null) return;
    for (const connStatus of connStatusArr) {
        const [, setter] = getOrCreateConnStatusPair(connStatus.connection);
        setter(connStatus);
    }
}

export function subscribeToConnEvents() {
    waveEventSubscribe({
        eventType: WpsEvent.ConnChange,
        handler: (event: WaveEvent) => {
            try {
                const connStatus = event.data as ConnStatus;
                if (connStatus == null || isBlank(connStatus.connection)) return;
                console.log("connstatus update", connStatus);
                const [, setter] = getOrCreateConnStatusPair(connStatus.connection);
                setter(connStatus);
            } catch (e) {
                console.log("connchange error", e);
            }
        },
    });
}

function makeDefaultConnStatus(conn: string, connected: boolean, hasconnected: boolean): ConnStatus {
    return {
        connection: conn,
        connected,
        error: null,
        status: connected ? "connected" : "disconnected",
        hasconnected,
        activeconnnum: 0,
    };
}

function getOrCreateConnStatusPair(conn: string): [() => ConnStatus, (v: ConnStatus) => void] {
    const map = connStatusMap();
    let pair = map.get(conn);
    if (pair == null) {
        const initial =
            isBlank(conn) || conn.startsWith("aws:")
                ? makeDefaultConnStatus(conn, true, true)
                : makeDefaultConnStatus(conn, false, false);
        const [get, set] = createSignal<ConnStatus>(initial);
        pair = [get, set];
        const newMap = new Map(map);
        newMap.set(conn, pair);
        setConnStatusMap(newMap);
    }
    return pair;
}

export function getConnStatusAtom(conn: string): () => ConnStatus {
    return getOrCreateConnStatusPair(conn)[0];
}
