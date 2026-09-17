// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fleet control — select, broadcast, and bulk-act on many agents at once.
// See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md and
// agentmux-srv/src/server/app_api/fleet.rs (the RPC handlers this binds to).

import { RpcClient } from "../rpc-client";

// Every shape below is GENERATED from its Rust definition by ts-rs and
// re-exported here under the name the frontend already used, so the public
// import surface (`@/app/store/rpc-api`) is unchanged. They were previously
// four hand-written `export interface` declarations plus five inline anonymous
// request shapes. See docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4
// step 2.
//
// `FleetStagePlan` is an alias: the Rust type is `StagePlanInput` (it is an
// input-only shape), but the frontend has always called it `FleetStagePlan`
// and `swarm-model.ts` imports that name. Renaming the Rust type to match
// would churn the server for a frontend spelling, so the alias lives here.
export type { FleetActionResult } from "@/types/rpc/FleetActionResult";
export type { FleetActionFailure } from "@/types/rpc/FleetActionFailure";
export type { FleetGroup } from "@/types/rpc/FleetGroup";
export type { StagePlanInput as FleetStagePlan } from "@/types/rpc/StagePlanInput";

import type { CommandFleetBroadcastData } from "@/types/rpc/CommandFleetBroadcastData";
import type { CommandFleetBulkStopData } from "@/types/rpc/CommandFleetBulkStopData";
import type { CommandFleetGroupCreateData } from "@/types/rpc/CommandFleetGroupCreateData";
import type { CommandFleetGroupListData } from "@/types/rpc/CommandFleetGroupListData";
import type { CommandFleetGroupUpdateData } from "@/types/rpc/CommandFleetGroupUpdateData";
import type { CommandFleetGroupDeleteData } from "@/types/rpc/CommandFleetGroupDeleteData";
import type { FleetActionResult as FleetActionResultT } from "@/types/rpc/FleetActionResult";
import type { FleetGroup as FleetGroupT } from "@/types/rpc/FleetGroup";
import type { FleetGroupListResult } from "@/types/rpc/FleetGroupListResult";
import type { FleetGroupDeleteResult } from "@/types/rpc/FleetGroupDeleteResult";

export const FleetApi = {
    // Broadcasts `message` to every block_id in `targets`, one signed jekt
    // per target (source_agent absent — the human/Swarm-UI path; see
    // fleet.rs's module doc comment). Always returns per-target detail,
    // never a single bool.
    FleetBroadcastCommand(
        client: RpcClient,
        data: CommandFleetBroadcastData,
        opts?: RpcOpts,
    ): Promise<FleetActionResultT> {
        return client.rpcCall("fleet.broadcast", data, opts);
    },

    // Stops every block_id in `targets`. `staged` caps blast radius on a
    // bad selection — see FleetStagePlan's fields.
    FleetBulkStopCommand(
        client: RpcClient,
        data: CommandFleetBulkStopData,
        opts?: RpcOpts,
    ): Promise<FleetActionResultT> {
        return client.rpcCall("fleet.bulk-stop", data, opts);
    },

    FleetGroupCreateCommand(
        client: RpcClient,
        data: CommandFleetGroupCreateData,
        opts?: RpcOpts,
    ): Promise<FleetGroupT> {
        return client.rpcCall("fleet.group.create", data, opts);
    },

    FleetGroupListCommand(
        client: RpcClient,
        data: CommandFleetGroupListData,
        opts?: RpcOpts,
    ): Promise<FleetGroupListResult> {
        return client.rpcCall("fleet.group.list", data, opts);
    },

    FleetGroupUpdateCommand(
        client: RpcClient,
        data: CommandFleetGroupUpdateData,
        opts?: RpcOpts,
    ): Promise<FleetGroupT> {
        return client.rpcCall("fleet.group.update", data, opts);
    },

    FleetGroupDeleteCommand(
        client: RpcClient,
        data: CommandFleetGroupDeleteData,
        opts?: RpcOpts,
    ): Promise<FleetGroupDeleteResult> {
        return client.rpcCall("fleet.group.delete", data, opts);
    },
};
