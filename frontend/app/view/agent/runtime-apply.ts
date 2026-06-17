// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * applyRuntimeChange — the single way to apply a runtime config change
 * (model / effort / permission) so it takes effect on the live agent.
 *
 * Persistent controllers (Claude stream-json) keep ONE CLI process alive
 * across turns, so the model/effort/permission flags are baked in at spawn —
 * a meta-only `agent:runtime` write silently no-ops for them. We therefore
 * rebuild `cmd:args` (via the same `buildRuntimeArgs`) and force a controller
 * restart; the persistent controller resumes via `--resume`, preserving the
 * conversation. Subprocess controllers re-spawn per turn, so the meta write
 * alone is enough there.
 *
 * Shared by the `/model`·`/effort`·`/mode` slash commands
 * (`commands/global/runtime.ts`) AND the GUI control-bar dropdowns
 * (`components/AgentControlBar.tsx`). Previously the persistent rebuild+restart
 * lived only in the slash path (#1503), so the GUI dropdown silently failed to
 * change the model on a running Claude agent — this helper unifies both.
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
 * `cmd:args` + force-restart so the change applies to the running agent. May
 * throw on RPC failure — callers decide how to surface it.
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
