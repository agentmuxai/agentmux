// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The names a folder holds, listed through srv: what Undo for a move checks before and after (files-undo-move.ts). */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

const PAGE = 1000;

/** Which connection and block a listing is for (the model's own). */
export type ListScope = { connection?: string; block_id?: string };

/** The lower-case names in `dir`, or null when it can't be listed or is too big to be sure of. */
export async function namesIn(dir: string, scope: ListScope): Promise<Set<string> | null> {
    const names = new Set<string>();
    let cursor: string | undefined;
    try {
        do {
            const res = await RpcApi.FsListCommand(TabRpcClient, { path: dir, cursor, limit: PAGE, ...scope });
            if (res.error) return null;
            for (const e of res.entries) names.add(e.name.toLowerCase());
            cursor = res.cursor ?? undefined;
            if (names.size > 20_000) return null;
        } while (cursor);
    } catch {
        return null;
    }
    return names;
}

/** The lower-case names a folder holds, "file" for something that isn't a folder, or null when that isn't certain. */
export async function childNamesOf(path: string, scope: ListScope): Promise<Set<string> | "file" | null> {
    try {
        const res = await RpcApi.FsListCommand(TabRpcClient, { path, limit: PAGE, ...scope });
        if (res.error) return res.error.kind === "not_a_directory" ? "file" : null;
        if (res.cursor) return null;
        return new Set(res.entries.map((e) => e.name.toLowerCase()));
    } catch {
        return null;
    }
}
