// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The container status store (container-status.ts) on srv: `GET /api/v1/containers`, the `container-status` and
// `container-runtime` events, and one instance for the window, made by its first reader.

import { authHeaders } from "@/app/store/auth-headers";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import type { ContainersList } from "@/types/rpc/ContainersList";
import { getWebServerEndpoint } from "@/util/endpoints";
import { createRoot } from "solid-js";
import { createContainerStatusStore, type ContainerStatusStore } from "./container-status";

/** The store on srv. */
export function createLiveContainerStatusStore(): ContainerStatusStore {
    return createContainerStatusStore({
        read: async () => {
            const res = await fetch(`${getWebServerEndpoint()}/api/v1/containers`, { headers: authHeaders() });
            if (!res.ok) throw new Error(`containers: ${res.status}`);
            return (await res.json()) as ContainersList;
        },
        onContainer: (handler) => onMuxEvent(WpsEvent.ContainerStatusChanged, handler),
        onRuntime: (handler) => onMuxEvent(WpsEvent.ContainerRuntimeChanged, handler),
        every: (ms, tick) => {
            const timer = setInterval(tick, ms);
            return () => clearInterval(timer);
        },
    });
}

let shared: ContainerStatusStore | undefined;

/** The window's store, made on first use and kept for the life of the window. */
export function sharedContainerStatus(): ContainerStatusStore {
    if (!shared) {
        shared = createRoot(() => createLiveContainerStatusStore());
        void shared.load();
    }
    return shared;
}
