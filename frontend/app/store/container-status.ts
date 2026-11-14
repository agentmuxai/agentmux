// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// This machine's agent containers and the container runtime's own state, as srv reports them (srv
// `backend/container_status.rs`, docs/specs/SPEC_CONTAINER_STATUS_IN_THE_FRONTEND_2026_11_14.md): names
// and states only. `GET /api/v1/containers` gives what is there, `container-status` carries a change to
// one container (`gone` once, when it is removed) and `container-runtime` a change of the runtime. A
// stream can miss something, so the route is read again every minute.

import type { ContainerRuntimeStatus } from "@/types/rpc/ContainerRuntimeStatus";
import type { ContainersList } from "@/types/rpc/ContainersList";
import type { ContainerStatus } from "@/types/rpc/ContainerStatus";
import { createSignal, type Accessor } from "solid-js";

/** How often the list is read again, for a change the event stream missed. */
const RECONCILE_EVERY_MS = 60_000;

export interface ContainerStatusDeps {
    /** `GET /api/v1/containers`. */
    read: () => Promise<ContainersList>;
    onContainer: (handler: (status: ContainerStatus) => void) => () => void;
    onRuntime: (handler: (status: ContainerRuntimeStatus) => void) => () => void;
    /** Every `ms`, until the returned stop is called. */
    every: (ms: number, tick: () => void) => () => void;
}

export interface ContainerStatusStore {
    /** This machine's agent containers, by name. */
    containers: Accessor<readonly ContainerStatus[]>;
    /** The container runtime's state; "not available" until the first read. */
    runtime: Accessor<ContainerRuntimeStatus>;
    /** Read the list, and follow the events. */
    load: () => Promise<void>;
    dispose: () => void;
}

/** The list with one change applied: a removed container leaves it, any other replaces its row or joins. */
export function withContainer(list: readonly ContainerStatus[], status: ContainerStatus): ContainerStatus[] {
    const rest = list.filter((c) => c.name !== status.name);
    if (status.state === "gone") return rest;
    return [...rest, status].sort((a, b) => a.name.localeCompare(b.name));
}

const NONE: ContainerRuntimeStatus = { available: false, at_ms: 0 };

export function createContainerStatusStore(deps: ContainerStatusDeps): ContainerStatusStore {
    const [containers, setContainers] = createSignal<readonly ContainerStatus[]>([]);
    const [runtime, setRuntime] = createSignal<ContainerRuntimeStatus>(NONE);
    // What the events said while a read was out: the read is older than they are.
    let meanwhile: ContainerStatus[] | null = null;
    let runtimeMeanwhile: ContainerRuntimeStatus | null = null;
    let stops: Array<() => void> = [];
    let disposed = false;

    const read = async () => {
        meanwhile = [];
        runtimeMeanwhile = null;
        try {
            const list = await deps.read();
            if (disposed) return;
            setContainers(meanwhile.reduce(withContainer, [...list.containers]));
            setRuntime(runtimeMeanwhile ?? list.runtime);
        } catch (e) {
            // srv older than the route, or unreachable: keep what there is, and try again on the next tick.
            console.warn("[container-status] reading the containers failed", e);
        } finally {
            meanwhile = null;
            runtimeMeanwhile = null;
        }
    };

    return {
        containers,
        runtime,
        load: async () => {
            if (stops.length > 0 || disposed) return;
            stops = [
                deps.onContainer((status) => {
                    meanwhile?.push(status);
                    setContainers((list) => withContainer(list, status));
                }),
                deps.onRuntime((status) => {
                    if (meanwhile) runtimeMeanwhile = status;
                    setRuntime(status);
                }),
                deps.every(RECONCILE_EVERY_MS, () => void read()),
            ];
            await read();
        },
        dispose: () => {
            disposed = true;
            for (const stop of stops) stop();
            stops = [];
        },
    };
}
