// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * applyRuntimeChange — the single way to apply a runtime config change
 * (model / effort / permission) so it takes effect on the live agent.
 *
 * For persistent controllers (Claude stream-json), flags are baked in at spawn
 * time. The process stays alive between turns and never re-reads cmd:args on
 * its own, so we rebuild cmd:args AND forcerestart — killing the idle process
 * so the next send_message spawns with the new flags.
 *
 * forcerestart is safe when the agent is idle (STATUS_DONE), and — since
 * 2026-08-30 — safe mid-turn too, because srv defers it.
 *
 * This comment previously claimed that a mid-turn forcerestart was survivable
 * because "the pane recovers via TurnReset (slash path) or has no TurnStart
 * outstanding (UI dropdown path)". Neither was true: `commands/global/runtime.ts`
 * contains no TurnReset, and "no TurnStart outstanding" only holds when the pane
 * is idle — the case the sentence had already excluded. Both paths fell through
 * to nothing, and the kill destroyed the user's in-flight message outright (the
 * turn simply went silent — diagnosed live on AgentX, 2026-08-28).
 *
 * The fix lives in srv rather than here, so it covers every caller of this
 * function and survives a pane close: `resync_controller`'s forced-replace path
 * asks a persistent controller to restart itself at the end of the current turn
 * (`PersistentSubprocessController::request_restart_when_idle`) instead of
 * tearing it down mid-flight. Nothing is lost by waiting — these flags are baked
 * in at spawn, so they could never have applied to the turn already running.
 *
 * Shared by the `/model`·`/effort`·`/mode` slash commands
 * (`commands/global/runtime.ts`) AND the GUI control-bar dropdowns
 * (`components/AgentControlBar.tsx`).
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as MOS from "@/app/store/mos";
import { staticTabId } from "@/app/store/global";
import { buildPaneArgs, getRuntimeConfig } from "./buildRuntimeArgs";
import { modelTakesEffort } from "./runtime-capabilities";
import { isPersistentLaunch, parseProviderFlags, PROVIDER_FLAGS_META_KEY, withoutOverriddenFlags } from "./launch-args";
import type { AgentRuntimeConfig } from "./types";
import type { ProviderDefinition } from "./providers";

/**
 * Persist `updated` to `agent:runtime` and, for persistent controllers, rebuild
 * `cmd:args` + forcerestart so the change applies immediately. May throw on RPC
 * failure — callers decide how to surface it.
 */
export async function applyRuntimeChange(
    blockId: string,
    provider: ProviderDefinition | undefined,
    updated: AgentRuntimeConfig,
    /**
     * The block's own `meta`. Two things are read from it, and both are
     * REQUIRED for correctness — it takes the whole object rather than one
     * extracted field so a third such fact doesn't mean a third parameter:
     *
     * - `agentMode` ("host" / "container"). This function used to branch on
     *   `provider.controllerType === "persistent"` alone — an unmigrated copy
     *   of the rule `launch-args.ts` now owns (reagent P1 on PR #2867). On a
     *   container agent that rewrote `--input-format stream-json` straight back
     *   into persisted `cmd:args` on every /model, /effort or /mode change,
     *   undoing the launch-time fix, and forced a controller restart the
     *   container path never needed.
     * - `agent:provider_flags`. The rebuild below starts from the provider
     *   catalog, so without reapplying these the user's flags are dropped from
     *   `cmd:args` by the first runtime change (#2872).
     *
     * Optional only so callers predating it keep compiling.
     */
    blockMeta?: Record<string, unknown>,
): Promise<void> {
    const agentMode = blockMeta?.["agentMode"] as string | undefined;
    const oref = MOS.makeORef("block", blockId);
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref,
        meta: { "agent:runtime": updated },
    });

    // Container agents are one-shot per `docker exec`, so they neither take
    // persistent args nor need the restart — `input.rs` reads `cmd:args` fresh
    // from block meta on every turn, so a runtime change applies to the next
    // one with no controller churn at all.
    if (provider && isPersistentLaunch(provider, agentMode)) {
        // `--fork-session` is deliberately not reapplied — it is a one-shot
        // launch intent, unlike provider_flags which describe how this agent
        // always runs. See withProviderFlags' own doc.
        const updatedArgs = buildPaneArgs(provider, agentMode, updated, blockMeta?.[PROVIDER_FLAGS_META_KEY]);
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref,
            meta: { "cmd:args": updatedArgs },
        });
        await RpcApi.ControllerResyncCommand(TabRpcClient, {
            tabid: staticTabId(),
            blockid: blockId,
            forcerestart: true,
        });
    }
}

/**
 * How long after its last write a block's own record of the runtime config is
 * preferred over the block's meta. The write has to round-trip into the block's
 * meta before a reader sees it; a change made inside that window must build on
 * what was just written, not on the stale read.
 */
const META_CATCH_UP_MS = 3000;

interface RuntimeChain {
    /** Tail of the block's change queue; each change runs after the previous. */
    tail: Promise<void>;
    /** Changes queued or running. */
    pending: number;
    /** The config (and the pane's provider flags) the last successful change wrote, with when. */
    last?: { config: AgentRuntimeConfig; flags: string; at: number };
}

const chains = new Map<string, RuntimeChain>();

/** Test hook: forget every block's queue. */
export function __resetRuntimeApply(): void {
    chains.clear();
}

/**
 * Change part of a pane's runtime config (model, effort or permission mode)
 * and apply it. THE entry point for the Runtime menu and the slash commands.
 *
 * Changes to one pane are applied one at a time, in the order they were made,
 * each built on the one before it. Callers used to build the new config from
 * the block's meta at click time and call `applyRuntimeChange`; the Runtime
 * panel stays open across selections, so a second change could read the meta
 * before the first one's write had come back, and its write undid the first —
 * in `agent:runtime` and in `cmd:args` alike, so the pane ran a combination
 * nobody had asked for.
 *
 * `getMeta` is read when the change RUNS, not when it is requested.
 * Resolves with the config that was applied; rejects with the underlying error
 * (callers decide how to show it). A failed change is not carried into the next
 * one: the next builds on what was last really applied, or on the block's meta.
 */
export function patchRuntime(
    blockId: string,
    provider: ProviderDefinition | undefined,
    patch: Partial<AgentRuntimeConfig>,
    getMeta: () => Record<string, unknown> | undefined,
): Promise<AgentRuntimeConfig> {
    const chain = chains.get(blockId) ?? { tail: Promise.resolve(), pending: 0 };
    chains.set(blockId, chain);
    chain.pending++;

    const run = chain.tail.then(async () => {
        const meta = getMeta();
        const recent = chain.last && Date.now() - chain.last.at < META_CATCH_UP_MS ? chain.last : undefined;
        const updated: AgentRuntimeConfig = { ...(recent?.config ?? getRuntimeConfig(meta)), ...patch };

        // A pick has to win over the agent definition's own flags. They are
        // appended after the runtime's, so a `--model` among them would
        // otherwise override the pick: the menu would show one model and the
        // process run another. Take the replaced flags out of THIS PANE'S copy;
        // the agent definition is untouched.
        const current = recent?.flags ?? parseProviderFlags(meta?.[PROVIDER_FLAGS_META_KEY]).join(" ");
        const flags = withoutOverriddenFlags(current, {
            model: patch.model !== undefined,
            // A model that takes no --effort (Haiku) must not be left with the
            // definition's: it is appended after the runtime's flags and Haiku
            // answers HTTP 400 on it (ReAgent P1 on #4161).
            effort: patch.effort !== undefined || (patch.model !== undefined && !modelTakesEffort(patch.model)),
            permissionMode: patch.permissionMode !== undefined,
        });
        if (flags !== current) {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: MOS.makeORef("block", blockId),
                // `MetaType` is a closed list that doesn't name this key; the
                // launch path writes it as a plain record too.
                meta: { [PROVIDER_FLAGS_META_KEY]: flags } as MetaType,
            });
        }

        await applyRuntimeChange(blockId, provider, updated, { ...meta, [PROVIDER_FLAGS_META_KEY]: flags });
        chain.last = { config: updated, flags, at: Date.now() };
        return updated;
    });
    chain.tail = run.then(
        () => undefined,
        () => undefined,
    );
    const done = () => {
        if (--chain.pending !== 0) return;
        // Idle: forget the block once its last write is old enough that its
        // meta can be trusted again, so a long session doesn't grow this map.
        setTimeout(() => {
            if (chain.pending === 0 && chains.get(blockId) === chain) chains.delete(blockId);
        }, META_CATCH_UP_MS + 100);
    };
    run.then(done, done);
    return run;
}
