// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Session-lifecycle actions (archive / restore / export) and the meta reads
 * that gate them.
 *
 * Extracted from `AgentControlBar`, which used to own all of this inside the
 * Shell drawer. Two surfaces need them now, so neither can own them:
 * `AgentSessionNotices` (the banners, above the composer) and
 * `AgentSessionStats` (the context-stats popover) — see
 * `docs/specs/SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md` §3.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as MOS from "@/app/store/mos";

/** Line count at/above which the session is "large" and worth archiving. */
export const LARGE_SESSION_THRESHOLD = 500_000;

export type BlockAccessor = () => Block | undefined;

export const sessionLineCount = (blockAtom: BlockAccessor): number =>
    (blockAtom()?.meta?.["session:line_count"] as number | undefined) ?? 0;

export const sessionArchivedAt = (blockAtom: BlockAccessor): number =>
    (blockAtom()?.meta?.["session:archived_at"] as number | undefined) ?? 0;

export const isSessionArchived = (blockAtom: BlockAccessor): boolean =>
    sessionArchivedAt(blockAtom) > 0;

export const isLargeSession = (blockAtom: BlockAccessor): boolean =>
    !isSessionArchived(blockAtom) && sessionLineCount(blockAtom) >= LARGE_SESSION_THRESHOLD;

export const sessionLastActivityMs = (blockAtom: BlockAccessor): number =>
    (blockAtom()?.meta?.["session:last_activity_ms"] as number | undefined) ?? 0;

export const wasInterrupted = (blockAtom: BlockAccessor): boolean =>
    (blockAtom()?.meta?.["session:was_interrupted"] as boolean | undefined) === true;

export const resumeFailed = (blockAtom: BlockAccessor): boolean =>
    (blockAtom()?.meta?.["session:resume_failed"] as boolean | undefined) === true;

export async function archiveSession(blockId: string): Promise<void> {
    await RpcApi.SessionArchiveCommand(TabRpcClient, { block_id: blockId });
}

export async function restoreSession(blockId: string): Promise<void> {
    await RpcApi.SessionRestoreCommand(TabRpcClient, { block_id: blockId });
}

/** Fetches the session transcript and saves it as a `.jsonl` download. */
export async function exportSession(blockId: string): Promise<void> {
    const result = await RpcApi.SessionExportCommand(TabRpcClient, { block_id: blockId });
    // Decode base64 and trigger a browser download
    const raw = atob(result.content);
    const bytes = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
    const blob = new Blob([bytes], { type: "application/x-ndjson" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `session-${blockId.slice(0, 8)}-${Date.now()}.jsonl`;
    a.click();
    URL.revokeObjectURL(url);
}

/** Clears a one-shot session notice flag (`session:was_interrupted` etc.). */
export async function clearSessionFlag(blockId: string, key: string): Promise<void> {
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref: MOS.makeORef("block", blockId),
        meta: { [key]: null } as MetaType,
    });
}
