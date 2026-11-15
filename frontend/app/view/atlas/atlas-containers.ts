// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * This machine's agent containers on the Atlas map (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.4), from srv's
 * list (`frontend/app/store/container-status.ts`): the container of each container agent, the ones no agent uses,
 * and what to say when the container runtime itself is not running. Pure.
 */

import type { ContainerRuntimeStatus } from "@/types/rpc/ContainerRuntimeStatus";
import type { ContainerStatus } from "@/types/rpc/ContainerStatus";
import { isContainerRuntime } from "@/util/runtime-kind";
import type { AtlasAgent, AtlasContainer, AtlasOrphan } from "./atlas-graph";

/** The agent runs in a container, as far as its machine says where its agents run. */
export function inContainer(agent: Pick<AtlasAgent, "runtime">): boolean {
    return agent.runtime != null && isContainerRuntime(agent.runtime);
}

/** A container as the map shows it; null for one that is gone, or in a state this build does not know. */
function containerOf(status: ContainerStatus): AtlasContainer | null {
    const { state } = status;
    if (state !== "starting" && state !== "running" && state !== "stopped") return null;
    return { name: status.name, state, image: status.image, sinceMs: status.since_ms };
}

/** Each agent's container, by the agent's id (the runtime's: `AtlasAgent.runtimeId`). */
export function containersByAgent(list: readonly ContainerStatus[]): Map<string, AtlasContainer> {
    const map = new Map<string, AtlasContainer>();
    for (const status of list) {
        const container = containerOf(status);
        if (container && status.agent_id) map.set(status.agent_id, container);
    }
    return map;
}

/** The containers no agent uses, as nodes of `machine`, by name. */
export function orphanContainers(list: readonly ContainerStatus[], machine: string): AtlasOrphan[] {
    return list
        .flatMap((status) => {
            const container = status.agent_id ? null : containerOf(status);
            return container ? [{ ...container, key: `container:${status.name}`, machine }] : [];
        })
        .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * What to say of this machine when its container runtime is not running and that matters: one of its agents runs
 * in a container, or a container is listed. Nothing before the first report (`at_ms` 0), and nothing on a machine
 * that uses no containers.
 */
export function containerRuntimeNote(
    runtime: ContainerRuntimeStatus,
    agents: readonly AtlasAgent[],
    containers: readonly ContainerStatus[]
): string | undefined {
    if (runtime.available || runtime.at_ms <= 0) return undefined;
    const used = containers.length > 0 || agents.some((a) => !a.remote && inContainer(a));
    return used ? "container runtime not running" : undefined;
}
