// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Archive the conversation, then return the pane to the picker. The archives
 * must land first: a first spawn continues whatever conversation the pane
 * would render (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §5 P0a), so an
 * unarchived one would simply reopen. Each archive runs even if an earlier
 * one fails, and a failure still returns to the picker.
 */
export async function archiveThenReturnToPicker(
    archives: ReadonlyArray<() => Promise<unknown>>,
    backToPicker: () => Promise<void>,
    onArchiveError: (error: unknown) => void
): Promise<void> {
    for (const archive of archives) {
        try {
            await archive();
        } catch (e) {
            onArchiveError(e);
        }
    }
    await backToPicker();
}

/**
 * The archives a "new session" needs. The pane's own transcript
 * (`session:archive`) is what a relaunch in this same pane would continue;
 * the agent's global zone (`agent:session:archive`) is what a different pane
 * opening this agent would continue. A pane without an agent id has no
 * global zone.
 */
export function newSessionArchives(
    blockId: string,
    definitionId: unknown,
    archiveBlock: (blockId: string) => Promise<unknown>,
    archiveAgent: (definitionId: string) => Promise<unknown>
): Array<() => Promise<unknown>> {
    const archives: Array<() => Promise<unknown>> = [() => archiveBlock(blockId)];
    if (typeof definitionId === "string" && definitionId !== "") {
        archives.push(() => archiveAgent(definitionId));
    }
    return archives;
}
