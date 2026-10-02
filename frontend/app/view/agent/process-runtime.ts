// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the agent's process was actually given, against what the Runtime menu
 * shows.
 *
 * The menu renders `agent:runtime` — what was REQUESTED. The process runs on the
 * argv it was spawned with, which is a separate record written by separate
 * code; when they differ the menu used to look fine while the agent ran on
 * something else (a resumed agent on the CLI default, Opus, under a menu that
 * read Sonnet — docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md).
 * srv publishes the runtime flags of the argv it spawned each process with
 * (`agentruntime`, crates/srv/src/backend/agent_runtime.rs); this module says
 * whether the menu and that agree.
 *
 * "What the menu should have produced" is computed with the SAME function that
 * builds the args (`buildPaneArgs`), not by comparing raw values: the args
 * deliberately translate some selections (a persistent agent's `bypass` becomes
 * `--permission-mode default`, Haiku takes no `--effort`), and comparing the
 * raw selection would flag every normal pane.
 */

import { onCleanup, onMount, createSignal, type Accessor } from "solid-js";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import * as MOS from "@/app/store/mos";
import { buildPaneArgs } from "./buildRuntimeArgs";
import type { LaunchArgsProvider } from "./launch-args";
import type { AgentRuntimeConfig } from "./types";

/** The runtime flags of an argv; `undefined` where the argv has none (the CLI default). */
export interface RuntimeFlags {
    model?: string;
    effort?: string;
    permissionMode?: string;
}

/** The `agentruntime` event, as the menu uses it. */
export interface ProcessRuntime extends RuntimeFlags {
    /** A process is alive. When it isn't, there is nothing to compare against. */
    running: boolean;
    /** A restart is on its way (a change arrived mid-turn): not a mismatch yet. */
    restartPending: boolean;
    /**
     * What the CLI itself says it is using (`get_settings`): the alias resolved to
     * a concrete model id, and the effort in force. Absent until it answers, for
     * a CLI that cannot, and for an effort the model does not take (Haiku).
     * Unlike the flags above this is not what was ASKED but what is IN FORCE.
     */
    effectiveModel?: string;
    effectiveEffort?: string;
}

/**
 * The runtime flags in `args`. Mirrors `spawn_runtime_from_args` in
 * agent_runtime.rs: the last occurrence of a repeated flag wins;
 * `--dangerously-skip-permissions` reads as `bypass`.
 */
export function runtimeFlagsFromArgs(args: readonly string[]): RuntimeFlags {
    const out: RuntimeFlags = {};
    for (let i = 0; i < args.length; i++) {
        const a = args[i];
        const value = args[i + 1];
        if ((a === "--model" || a === "-m") && value !== undefined) {
            out.model = value;
            i++;
        } else if (a === "--effort" && value !== undefined) {
            out.effort = value;
            i++;
        } else if (a === "--permission-mode" && value !== undefined) {
            out.permissionMode = value;
            i++;
        } else if (a === "--dangerously-skip-permissions") {
            out.permissionMode = "bypass";
        } else if (a.startsWith("--model=")) out.model = a.slice("--model=".length);
        else if (a.startsWith("--effort=")) out.effort = a.slice("--effort=".length);
        else if (a.startsWith("--permission-mode=")) out.permissionMode = a.slice("--permission-mode=".length);
    }
    return out;
}

/** Parse an `agentruntime` event's data; `null` if it isn't one. */
export function parseAgentRuntimeEvent(data: unknown): ProcessRuntime | null {
    if (!data || typeof data !== "object") return null;
    const d = data as Record<string, unknown>;
    if (typeof d.running !== "boolean") return null;
    const str = (v: unknown) => (typeof v === "string" && v !== "" ? v : undefined);
    return {
        running: d.running,
        restartPending: d.restart_pending === true,
        model: str(d.model),
        effort: str(d.effort),
        permissionMode: str(d.permission_mode),
        effectiveModel: str(d.effective_model),
        effectiveEffort: str(d.effective_effort),
    };
}

export type RuntimeAxis = "model" | "effort" | "permissionMode";

export interface AxisDrift {
    axis: RuntimeAxis;
    /** What the menu's selection produces. */
    wanted: string;
    /** What the process was given; `undefined` = no flag, so the CLI's own default. */
    running: string | undefined;
}

export type RuntimeAgreement =
    /** Nothing to judge: no process, no report yet, or a provider that reports none. */
    | { kind: "unknown" }
    | { kind: "agrees" }
    /** They differ, but a restart is already coming, so the selection will land. */
    | { kind: "pending"; drift: AxisDrift[] }
    /** They differ and nothing is going to fix it. */
    | { kind: "differs"; drift: AxisDrift[] };

/**
 * Whether the process runs what the menu says.
 *
 * Only axes the selection actually produces a flag for are judged: Haiku takes
 * no `--effort`, so there is no effort to be missing.
 */
export function compareRuntime(
    provider: (LaunchArgsProvider & { id: string }) | undefined,
    agentMode: string | undefined,
    requested: AgentRuntimeConfig,
    providerFlags: unknown,
    process: ProcessRuntime | undefined,
): RuntimeAgreement {
    if (!provider || !process || !process.running) return { kind: "unknown" };
    const wanted = runtimeFlagsFromArgs(buildPaneArgs(provider, agentMode, requested, providerFlags));
    const drift: AxisDrift[] = [];
    const axes: [RuntimeAxis, string | undefined, string | undefined][] = [
        ["model", wanted.model, process.model],
        ["effort", wanted.effort, process.effort],
        ["permissionMode", wanted.permissionMode, process.permissionMode],
    ];
    for (const [axis, want, run] of axes) {
        if (want !== undefined && want !== run) drift.push({ axis, wanted: want, running: run });
    }
    if (drift.length === 0) return { kind: "agrees" };
    return process.restartPending ? { kind: "pending", drift } : { kind: "differs", drift };
}

/**
 * The latest `agentruntime` report for one pane. The event is persisted by the
 * broker, so a menu that mounts after the process spawned still receives it.
 */
export function useProcessRuntime(blockId: string): Accessor<ProcessRuntime | undefined> {
    const [state, setState] = createSignal<ProcessRuntime | undefined>(undefined);
    onMount(() => {
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.AgentRuntime,
            scope: MOS.makeORef("block", blockId),
            handler: (event) => {
                const parsed = parseAgentRuntimeEvent((event as { data?: unknown })?.data);
                if (parsed) setState(parsed);
            },
        });
        onCleanup(() => unsub?.());
    });
    return state;
}
