// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where an agent runs: the one place the frontend reads `agentMode` (a pane), `agent_type` (a
 * definition) or a wire `kind` as "host or container". Mirrors srv's `RuntimeKind`
 * (crates/srv/src/backend/runtime_adapter.rs): the same two kinds, the same wire strings and the
 * same reading rules, under the names `of_agent_mode`, `of_agent_type`, `of_block_meta`, `parse`
 * and `is_container` there.
 *
 * A comparison with the string `"container"` anywhere else is a branch this module should answer;
 * `runtime-kind.test.ts` fails on a new one, as srv's `nothing_but_the_adapter_compares_a_mode_to_container`
 * does.
 */

export type RuntimeKind = "host" | "container";

const CONTAINER = "container";

/**
 * The runtime of a pane's `agentMode`: `"container"` is a container, anything else (`"host"`, the legacy
 * `"standalone"`, empty, absent or unknown) is the host, as srv has always read it.
 */
export function runtimeKindOfAgentMode(agentMode: unknown): RuntimeKind {
    return agentMode === CONTAINER ? "container" : "host";
}

/** The runtime of a definition's `agent_type`. The same values as `agentMode`. */
export function runtimeKindOfAgentType(agentType: unknown): RuntimeKind {
    return runtimeKindOfAgentMode(agentType);
}

/** The runtime of a pane, from its block meta. */
export function runtimeKindOfBlockMeta(meta: { agentMode?: unknown } | Record<string, unknown> | null | undefined): RuntimeKind {
    return runtimeKindOfAgentMode(meta?.["agentMode"]);
}

/**
 * The runtime a wire value names, `undefined` for anything but exactly `"host"` or `"container"`. For a
 * value that is validated (a stored choice, a row's runtime); the `runtimeKindOf…` functions read the
 * stored value, where anything else means the host.
 */
export function parseRuntimeKind(kind: unknown): RuntimeKind | undefined {
    return kind === "host" || kind === CONTAINER ? kind : undefined;
}

export function isContainerRuntime(kind: RuntimeKind): boolean {
    return kind === CONTAINER;
}

/** `runtimeKindOfAgentMode(agentMode)` is a container. */
export function isContainerAgentMode(agentMode: unknown): boolean {
    return isContainerRuntime(runtimeKindOfAgentMode(agentMode));
}

/** `runtimeKindOfAgentType(agentType)` is a container. */
export function isContainerAgentType(agentType: unknown): boolean {
    return isContainerRuntime(runtimeKindOfAgentType(agentType));
}
