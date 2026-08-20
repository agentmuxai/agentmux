// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fleet control — select, broadcast, and bulk-act on many agents at once.
// See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md and
// agentmux-srv/src/server/app_api/fleet.rs (the RPC handlers this binds to).

import { RpcClient } from "../rpc-client";

export interface FleetActionFailure {
    id: string;
    error: string;
}

export interface FleetActionResult {
    succeeded: string[];
    failed: FleetActionFailure[];
    aborted_early: boolean;
}

export interface FleetStagePlan {
    batch_size: number;
    max_fail_percentage: number;
}

export interface FleetGroup {
    id: string;
    name: string;
    member_ids: string[];
    created_at: number;
}

export const FleetApi = {
    // Broadcasts `message` to every block_id in `targets`, one signed jekt
    // per target (source_agent absent — the human/Swarm-UI path; see
    // fleet.rs's module doc comment). Always returns per-target detail,
    // never a single bool.
    FleetBroadcastCommand(
        client: RpcClient,
        data: { targets: string[]; message: string },
        opts?: RpcOpts,
    ): Promise<FleetActionResult> {
        return client.rpcCall("fleet.broadcast", data, opts);
    },

    // Stops every block_id in `targets`. `staged` caps blast radius on a
    // bad selection — see FleetStagePlan's fields.
    FleetBulkStopCommand(
        client: RpcClient,
        data: { targets: string[]; signal?: string; staged?: FleetStagePlan },
        opts?: RpcOpts,
    ): Promise<FleetActionResult> {
        return client.rpcCall("fleet.bulk-stop", data, opts);
    },

    FleetGroupCreateCommand(
        client: RpcClient,
        data: { name: string; member_ids: string[] },
        opts?: RpcOpts,
    ): Promise<FleetGroup> {
        return client.rpcCall("fleet.group.create", data, opts);
    },

    FleetGroupListCommand(
        client: RpcClient,
        data: Record<string, never>,
        opts?: RpcOpts,
    ): Promise<{ groups: FleetGroup[] }> {
        return client.rpcCall("fleet.group.list", data, opts);
    },

    FleetGroupUpdateCommand(
        client: RpcClient,
        data: { id: string; name?: string; member_ids?: string[] },
        opts?: RpcOpts,
    ): Promise<FleetGroup> {
        return client.rpcCall("fleet.group.update", data, opts);
    },

    FleetGroupDeleteCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean }> {
        return client.rpcCall("fleet.group.delete", data, opts);
    },
};
