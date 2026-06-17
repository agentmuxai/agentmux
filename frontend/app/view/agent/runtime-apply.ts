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
 * forcerestart is safe when the agent is idle (STATUS_DONE). If triggered
 * mid-turn the streaming turn is interrupted; the partial response stays in the
 * blockfile but the pane recovers via TurnReset (slash path) or has no
 * TurnStart outstanding (UI dropdown path).
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
): Promise<void> {
    const oref = WOS.makeORef("block", blockId);
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref,
        meta: { "agent:runtime": updated },
    });

    if (provider?.controllerType === "persistent") {
        const baseArgs = provider.persistentLaunchArgs ?? provider.launchArgs;
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
