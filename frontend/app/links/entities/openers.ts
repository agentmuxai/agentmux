// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The only place a kind opens anything (spec §4.2): thin adapters over the app's existing openers, until the
 * deep-links framework's typed wrappers (`openAgent`, `openTask`, `openUrl`) merge and replace each body. A kind's
 * `open` calls one of these and never opens a pane itself; the registry test reads the kinds' source to prove it.
 */

import type { Destination, OpenOptions, OpenResult } from "./types";

const OK: OpenResult = { ok: true };

async function attempt(run: () => Promise<void>): Promise<OpenResult> {
    try {
        await run();
        return OK;
    } catch (err) {
        return { ok: false, reason: err instanceof Error ? err.message : String(err) };
    }
}

export function openTaskEntity(dest: Extract<Destination, { kind: "task" }>, options: OpenOptions): Promise<OpenResult> {
    return attempt(async () => {
        if (dest.board) {
            const { openKanbanTask } = await import("@/app/util/open-kanban-task");
            await openKanbanTask(`${dest.board}#${dest.number}`);
            return;
        }
        const { openTaskLink } = await import("@/app/element/link-destinations");
        await openTaskLink(dest.number, options);
    });
}

/** An address opens in the person's own browser, by their click; a floating browser pane when asked. */
export function openUrlEntity(url: string, options: OpenOptions): Promise<OpenResult> {
    return attempt(async () => {
        if (options.floating) {
            const { openWebLinkFloating } = await import("@/app/element/link-destinations");
            await openWebLinkFloating(url);
            return;
        }
        const { openLink } = await import("@/app/store/misc-utils");
        await openLink(url);
    });
}

/** The agent destination ships with the deep-links framework; until then an agent link is drawn and says so. */
export function openAgentEntity(_dest: Extract<Destination, { kind: "agent" }>): Promise<OpenResult> {
    return Promise.resolve({ ok: false, reason: "Opening an agent from text is not available yet" });
}
