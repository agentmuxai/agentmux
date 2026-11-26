// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The only place a kind opens anything (spec §4.2): thin adapters over the app's existing openers, until the
 * deep-links framework's typed wrappers (`openAgent`, `openTask`, `openUrl`) merge and replace each body. A kind's
 * `open` calls one of these and never opens a pane itself; the registry test reads the kinds' source to prove it.
 */

import { asGesture } from "@/app/store/trail-hooks";
import { siteOf } from "./bare-domain";
import type { Destination, OpenOptions, OpenResult } from "./types";

const OK: OpenResult = { ok: true };

async function attempt(run: () => Promise<void>): Promise<OpenResult> {
    try {
        // A person's click on a link: one step of the window's trail of places.
        await asGesture(run);
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

/** An agent of this computer: its pane is shown, or opened, as the palette and the Swarm do. An agent of another
 *  computer waits for the deep-links framework's remote pane. */
export function openAgentEntity(dest: Extract<Destination, { kind: "agent" }>): Promise<OpenResult> {
    return attempt(async () => {
        const { getHostName } = await import("@/app/store/misc-utils");
        const here = getHostName().toLowerCase();
        if (dest.computer && here && dest.computer.toLowerCase() !== here) {
            throw new Error("That agent runs on another computer");
        }
        const { goToAgent } = await import("@/app/modals/palette-agents");
        await goToAgent({ agent_id: dest.id, name: dest.name });
    });
}

/** A file a message names, opened as the author's own folders allow: srv checks the path first and says why when it
 *  is not there, or not the author's. A container agent's file is opened by srv as a read-only copy; any other
 *  agent's is opened at the place srv resolved it to. */
export function openFileEntity(dest: Extract<Destination, { kind: "file" }>, options: OpenOptions): Promise<OpenResult> {
    return attempt(async () => {
        const { checkFileNow } = await import("../file-check-store");
        const found = await checkFileNow(dest.blockId, dest.path);
        if (found.state !== "found") {
            // A name that is also a web site is the site when srv says no such file is there; a check that could not be
            // made, or a path outside the folders, says its reason instead.
            const site = found.state === "missing" ? siteOf(dest.path) : null;
            if (site) {
                const opened = await openUrlEntity(site, options);
                if ("reason" in opened) throw new Error(opened.reason);
                return;
            }
            throw new Error(found.reason || "That file can't be opened.");
        }
        const { openPathLink } = await import("@/app/element/link-destinations");
        await openPathLink(dest.confined ? dest.path : found.resolved || dest.path, {
            floating: options.floating,
            owner: { blockId: dest.blockId, confined: dest.confined },
        });
    });
}
