// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * applyRuntimeChange — the single way to apply a runtime config change
 * (model / effort / permission) so it takes effect on the live agent.
 *
 * For persistent controllers (Claude stream-json), flags are baked in at spawn
 * time, so we also rebuild `cmd:args` — the next turn's spawn picks them up.
 * We do NOT forcerestart: "applies to next turn" is the correct semantic, and
 * a forcerestart creates a kill-race and can corrupt a streaming turn.
 *
 * Shared by the `/model`·`/effort`·`/mode` slash commands
 * (`commands/global/runtime.ts`) AND the GUI control-bar dropdowns
 * (`components/AgentControlBar.tsx`).
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as WOS from "@/app/store/wos";
import { buildRuntimeArgs } from "./buildRuntimeArgs";
import type { AgentRuntimeConfig } from "./types";
import type { ProviderDefinition } from "./providers";

/**
 * Persist `updated` to `agent:runtime` and, for persistent controllers, rebuild
 * `cmd:args` so the change applies at next spawn. May throw on RPC failure —
 * callers decide how to surface it.
 *
 * We intentionally do NOT forcerestart here. Persistent controllers re-read
 * `cmd:args` on every spawn, so the new model/effort/permission is already
 * baked in for the next turn — which is exactly what "applies to next turn"
 * means. A forcerestart would kill a potentially-streaming turn, leave the
 * blockfile with a partial response, and create a status-update race between
 * the old controller's async kill task and the new controller registration.
 * See docs/retros/RETRO_MODEL_SLASH_CMD_STUCK_2026_06_17.md.
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
    }
}
