// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Memory delivery — the hidden memory reinjection's claim against Claude
// Code's SessionStart hook (`memorydelivery:*`, agentmux-srv
// server/memory_delivery_handlers.rs). See
// docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2.

import { RpcClient } from "../rpc-client";

// GENERATED from their Rust definitions by ts-rs
// (crates/srv/src/backend/rpc_types/bundle.rs).
export type { CommandMemoryDeliveryClaimFallbackData } from "@/types/rpc/CommandMemoryDeliveryClaimFallbackData";
export type { MemoryDeliveryClaimFallbackResult } from "@/types/rpc/MemoryDeliveryClaimFallbackResult";
import type { CommandMemoryDeliveryClaimFallbackData } from "@/types/rpc/CommandMemoryDeliveryClaimFallbackData";
import type { MemoryDeliveryClaimFallbackResult } from "@/types/rpc/MemoryDeliveryClaimFallbackResult";

export const MemoryDeliveryApi = {
    /**
     * Whether the hidden reinjection should deliver this event (`deliver:
     * true`) or stand down because the SessionStart hook delivered it. The
     * sidecar may wait a few seconds for a hook delivery still in flight.
     */
    ClaimFallbackCommand(
        client: RpcClient,
        data: CommandMemoryDeliveryClaimFallbackData,
        opts?: RpcOpts,
    ): Promise<MemoryDeliveryClaimFallbackResult> {
        return client.rpcCall("memorydelivery:claim_fallback", data, opts);
    },
};
