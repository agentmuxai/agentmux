// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The block-meta keys the agent pane reads, as reactive accessors with their defaults in one
 * place. Reactive: a rename or a mode change shows without remounting the pane.
 */

import type { Accessor } from "solid-js";

type BlockLike = Accessor<{ meta?: Record<string, unknown> } | null | undefined>;

export function paneMeta(block: BlockLike, agentId: string) {
    const read = <T>(key: string): T | undefined => block()?.meta?.[key] as T | undefined;
    return {
        /** The lowercase provider catalog key; the agent's id when none is set. */
        providerKey: (): string => read<string>("agentProvider") ?? agentId,
        outputFormat: (): string => read<string>("agentOutputFormat") ?? "claude-stream-json",
        /** What to call this agent in the UI (the composer placeholder, jekt direction). */
        agentName: (): string => read<string>("agentName") ?? agentId,
        /** "host" or "container". */
        agentMode: (): string | undefined => read<string>("agentMode"),
        isContainer: (): boolean => read<string>("agentMode") === "container",
        cwd: (): string => read<string>("cmd:cwd") ?? "",
        sessionId: (): string | undefined => read<string>("agent:sessionid"),
        shellSubBlockId: (): string | undefined => read<string>("term:shellsubblockid"),
        shellHeight: (): number | undefined => read<number>("term:shellheight"),
    };
}
