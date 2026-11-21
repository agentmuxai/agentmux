// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The context a surface hands the resolver: who wrote the text, which repository and board the surface knows, and
 * the two indexes, which come from the window's roster and the published tasks, never from the surface.
 */

import { type Accessor, createMemo, createRoot } from "solid-js";
import { agentRoster } from "@/app/store/agent-names";
import { srvInfo } from "@/app/store/srv-info";
import { buildAgentIndex } from "./entities/indexes";
import type { AgentIndex, Author, LinkContext } from "./entities/types";
import { taskIndex } from "./task-index-store";

let agentVersion = 0;

let agentIndex: Accessor<AgentIndex> | undefined;

/** Rebuilt only when the roster changes; every surface reads the same index. Made on first use, so a window that
 *  never links anything never reads the roster. */
function sharedAgentIndex(): AgentIndex {
    agentIndex ??= createRoot(() =>
        createMemo(() => {
            agentVersion += 1;
            const entries = agentRoster().map((a) => ({ id: a.id, slug: a.slug ?? "", name: a.name ?? "" }));
            const host = srvInfo()?.hostName?.toLowerCase();
            return buildAgentIndex(entries, { localComputer: host || undefined, version: agentVersion });
        })
    );
    return agentIndex();
}

/** What a surface knows: the repository and board of the thing it shows, who wrote the text, and whether it is complete. */
export type SurfaceContext = { author?: Author; repo?: string; board?: string; final?: boolean };

/** Reactive: read inside a computation, it follows the roster and the tasks. */
export function linkContext(surface: SurfaceContext = {}): LinkContext {
    return {
        author: surface.author ?? { kind: "app" },
        agents: sharedAgentIndex(),
        tasks: taskIndex(),
        repo: surface.repo,
        board: surface.board,
        final: surface.final ?? true,
    };
}
