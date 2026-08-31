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
import * as WOS from "@/app/store/wos";
import { staticTabId } from "@/app/store/global";
import { buildRuntimeArgs } from "./buildRuntimeArgs";
import { isPersistentLaunch, selectLaunchArgs } from "./launch-args";
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
     * `block.meta["agentMode"]` — "host" or "container". REQUIRED for
     * correctness on container agents, defaulted only so existing callers
     * that predate it keep compiling.
     *
     * This function used to branch on `provider.controllerType === "persistent"`
     * alone — a third, unmigrated copy of the rule `launch-args.ts` now owns
     * (reagent P1 on PR #2867). On a container agent that rewrote
     * `--input-format stream-json` straight back into persisted `cmd:args` on
     * every /model, /effort or /mode change, undoing the launch-time fix, and
     * forced a controller restart the container path never needed.
     */
    agentMode?: string,
): Promise<void> {
    const oref = WOS.makeORef("block", blockId);
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref,
        meta: { "agent:runtime": updated },
    });

    // Container agents are one-shot per `docker exec`, so they neither take
    // persistent args nor need the restart — `input.rs` reads `cmd:args` fresh
    // from block meta on every turn, so a runtime change applies to the next
    // one with no controller churn at all.
    if (provider && isPersistentLaunch(provider, agentMode)) {
        const baseArgs = selectLaunchArgs(provider, agentMode);
        const updatedArgs = buildRuntimeArgs(baseArgs, updated, provider.id);
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
