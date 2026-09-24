// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Archive the agent's current conversation, then return the pane to the
 * picker. The archive must land first: a first spawn continues whatever
 * conversation the pane would render
 * (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §5 P0a), so an unarchived
 * one would simply reopen. A failed archive still returns to the picker.
 */
export async function archiveThenReturnToPicker(
    definitionId: unknown,
    archive: (definitionId: string) => Promise<unknown>,
    backToPicker: () => Promise<void>,
    onArchiveError: (error: unknown) => void
): Promise<void> {
    if (typeof definitionId === "string" && definitionId !== "") {
        try {
            await archive(definitionId);
        } catch (e) {
            onArchiveError(e);
        }
    }
    await backToPicker();
}
