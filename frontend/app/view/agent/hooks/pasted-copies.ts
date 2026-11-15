// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The copies a paste made in an agent's working folder (a container pane, or `attachments:enabled`
 * off), so one the person took back out of the message does not stay behind unseen.
 *
 * A paste there copies the file into the working folder and puts `@name` in the composer. srv makes
 * every such copy under a name that was free (an exclusive create), so each one is a file this paste
 * made and nothing the person or the agent had. If the person deletes the mention and sends, the
 * copy is no longer wanted: it goes to the Trash (recoverable), and a copy whose mention is still in
 * the message stays. Either way the list is emptied by the send, so a copy a sent message referred
 * to is never touched later.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { baseName } from "@/util/dnd";

const copiesByBlock = new Map<string, string[]>();

/** Remember the paths a paste just copied into this pane's working folder. */
export function rememberPastedCopies(blockId: string, paths: string[]): void {
    if (paths.length === 0) return;
    copiesByBlock.set(blockId, [...(copiesByBlock.get(blockId) ?? []), ...paths]);
}

/** The pasted copies not yet settled by a send, for tests. */
export function pendingPastedCopies(blockId: string): string[] {
    return copiesByBlock.get(blockId) ?? [];
}

/**
 * After a message is sent: trash each remembered copy whose `@name` is not in it, and forget them
 * all. A name that merely contains another (`@a.png2`) counts as a mention, so a doubtful case keeps
 * the file. A trash that fails leaves the file where it is, as before.
 */
export async function settlePastedCopies(blockId: string, sentMessage: string): Promise<void> {
    const copies = copiesByBlock.get(blockId);
    if (!copies?.length) return;
    copiesByBlock.delete(blockId);
    const unwanted = copies.filter((p) => !sentMessage.includes(`@${baseName(p)}`));
    if (unwanted.length === 0) return;
    try {
        await RpcApi.FsTrashCommand(TabRpcClient, { paths: unwanted });
    } catch {
        // Leave the files: an unwanted copy in the folder is the old behaviour, not a new failure.
    }
}
