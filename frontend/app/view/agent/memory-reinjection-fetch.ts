// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Real RPC-backed fetchers for SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_
 * COMPACTION_2026_09_22.md's `MemoryEntryInput[]` — the thin glue layer
 * between `memory-reinjection-controller.ts`'s injectable `fetchEntries`
 * and the actual RPC clients. Deliberately kept separate from the
 * controller (which has zero RPC dependencies of its own) so the
 * controller stays unit-testable with a fake; this file is the one place
 * that needs a real `RpcClient` and is verified by direct reading rather
 * than unit tests — it's thin, typed pass-through, not logic.
 *
 * Global Memory (`BundleApi.GlobalMemorySectionsCommand`) returns each
 * section's full text inline — no second round-trip needed. Personal/
 * native memory (`NativeMemoryApi.NativeMemoryListCommand`) returns
 * metadata only (`NativeMemoryFileMeta` — no content field), so each
 * non-index file needs a follow-up `NativeMemoryReadFileCommand` call.
 * `MEMORY.md` (`is_index: true`) is excluded — it's the index Claude Code
 * itself maintains, not curated content; §3.1's "full bodies" requirement
 * is about the actual memory entries, not their own table of contents.
 */

import { BundleApi } from "@/app/store/rpc-api/bundle";
import { NativeMemoryApi } from "@/app/store/rpc-api/native-memory";
import type { RpcClient } from "@/app/store/rpc-client";
import type { MemoryEntryInput } from "./memory-reinjection";

/**
 * Global Memory entries — the sections of the block agents' startup files
 * carry, Operator Config first, from the same store and formatter the
 * startup-file writers use (`globalmemory:sections`). Joined by
 * `GLOBAL_SECTION_SEPARATOR` they are that block byte for byte, so a
 * reinjection re-delivers exactly what the agent launched with. Operator
 * Config used to be left out here; with only Operator Config present, that
 * left nothing to send, and a fresh session got no reinjection at all
 * (SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §4, §7 P1).
 */
export async function fetchGlobalMemoryEntries(client: RpcClient, blockId?: string): Promise<MemoryEntryInput[]> {
    // The block lets srv leave out Operator Config meant for another agent
    // kind, as that pane's startup file does.
    const sections = await BundleApi.GlobalMemorySectionsCommand(client, { block_id: blockId });
    return sections.map((s) => ({
        label: `${s.is_system ? "[AgentMux System]" : "[Workspace]"} ${s.name}`,
        source: "global" as const,
        body: s.text,
        sizeBytes: s.size_bytes,
    }));
}

/**
 * Personal (native) memory entries for one agent — every non-index file,
 * full body content via a follow-up read per file.
 */
export async function fetchPersonalMemoryEntries(client: RpcClient, agentId: string): Promise<MemoryEntryInput[]> {
    const { files } = await NativeMemoryApi.NativeMemoryListCommand(client, { agent_id: agentId });
    const contentFiles = files.filter((f) => !f.is_index);

    // Sequential, not Promise.all: a memory directory is small (§3.4.1's real
    // measurement — tens of files at most on this machine today), and
    // sequential reads keep a single slow/failing file from cancelling every
    // other in-flight read the way Promise.all's fail-fast semantics would —
    // consistent with this feature's own "never silently drop content"
    // requirement (§3.1).
    const entries: MemoryEntryInput[] = [];
    for (const meta of contentFiles) {
        const { content } = await NativeMemoryApi.NativeMemoryReadFileCommand(client, {
            agent_id: agentId,
            filename: meta.filename,
        });
        if (content.trim().length === 0) continue;
        entries.push({
            label: meta.filename,
            source: "personal",
            body: content,
            sizeBytes: meta.size_bytes,
        });
    }
    return entries;
}

/** Combines both sources into the single entry list `shouldReinject`/`buildMemoryReinjectionNode` expect. */
export async function fetchMemoryReinjectionEntries(
    client: RpcClient,
    agentId: string,
    blockId?: string
): Promise<MemoryEntryInput[]> {
    const [global, personal] = await Promise.all([
        fetchGlobalMemoryEntries(client, blockId),
        fetchPersonalMemoryEntries(client, agentId),
    ]);
    return [...global, ...personal];
}
