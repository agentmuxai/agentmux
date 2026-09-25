// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryHistoryModel — view model for one native memory file's version
 * history: list, diff any two versions, revert to a prior version, plus the
 * file's current content. Mounted from two places — the agent's own Stash
 * "Memory" tab (`AgentNativeMemoryModal`) and Armory's Personal Memory full
 * view (`NativeMemoryFileView`) — both reading the identical
 * `agent:memory:history/diff/revert` RPCs, so there is one source of truth
 * and two entry points, per
 * docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3.
 *
 * The model itself now lives in memory-editor/memory-history-model.ts with a
 * pluggable data source (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3);
 * this is that model bound to the `agent:memory:*` RPCs.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { NativeMemoryVersionMeta } from "@/app/store/rpc-api";
import { MemoryHistoryModel, type MemoryHistorySource } from "@/app/view/memory-editor/memory-history-model";

export { orderVersionsOldestFirst, sourceLabel, sourceWarning } from "@/app/view/memory-editor/memory-history-model";

/** The `agent:memory:*` data source for one (agent, file). */
export function nativeMemoryHistorySource(
    agentId: string,
    filename: string,
): MemoryHistorySource<NativeMemoryVersionMeta> {
    // Plain `.then` chains rather than `async` wrappers: each extra `await`
    // layer is one more microtask before a result lands, and nothing here
    // needs one.
    return {
        listVersions: () =>
            RpcApi.NativeMemoryHistoryCommand(TabRpcClient, { agent_id: agentId, filename }).then((r) => r.versions),
        diff: (fromVersionId, toVersionId) =>
            RpcApi.NativeMemoryDiffCommand(TabRpcClient, {
                agent_id: agentId,
                from_version_id: fromVersionId,
                to_version_id: toVersionId,
            }).then((r) => r.diff),
        revert: (versionId) =>
            RpcApi.NativeMemoryRevertCommand(TabRpcClient, {
                agent_id: agentId,
                filename,
                target_version_id: versionId,
            }).then(() => undefined),
        readContent: () =>
            RpcApi.NativeMemoryReadFileCommand(TabRpcClient, { agent_id: agentId, filename }).then((r) => r.content),
    };
}

export class NativeMemoryHistoryModel extends MemoryHistoryModel<NativeMemoryVersionMeta> {
    readonly agentId: string;
    readonly filename: string;

    constructor(agentId: string, filename: string) {
        super(nativeMemoryHistorySource(agentId, filename));
        this.agentId = agentId;
        this.filename = filename;
    }
}
