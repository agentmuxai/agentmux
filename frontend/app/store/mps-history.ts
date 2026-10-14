// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A typed event's persisted history, read the way `onMuxEvent` delivers it live: as payloads
// of the event's Rust type (docs/specs/SPEC_CODEGEN_REMAINING_GAPS_2026_10_13.md). A typed event
// must be read this way, not with EventReadHistoryCommand (scripts/gen-mux-events.mjs --check).

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { isNoPayloadMuxEvent, type MuxEventPayloads } from "./mps-events.generated";

/** The last `maxitems` stored payloads of `event` in `scope`, oldest first. A stored event with no
 *  data is skipped, as `onMuxEvent` skips one, unless the event carries none by design. */
export async function readMuxEventHistory<K extends keyof MuxEventPayloads>(
    event: K,
    scope: string,
    maxitems: number
): Promise<MuxEventPayloads[K][]> {
    const events = await RpcApi.EventReadHistoryCommand(TabRpcClient, { event, scope, maxitems });
    return (events ?? [])
        .filter((e) => e.data != null || isNoPayloadMuxEvent(event))
        .map((e) => (e.data ?? null) as MuxEventPayloads[K]);
}
