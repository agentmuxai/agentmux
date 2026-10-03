// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Sessions on <host>" (SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md
// §7.6): the durable sessions AgentMux's helper holds on an SSH host, so a
// session no pane holds any more (an orphan) can be opened again or ended.

import { RpcApi } from "@/app/store/rpc-api";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { createBlock } from "@/app/store/block-layout-actions";
import { TabRpcClient } from "@/app/store/rpc-util";
import { formatBytes } from "@/util/format-bytes";

export type HostSessionActions = {
    /** Open the session in a new durable pane. */
    open: (id: string) => void;
    /** End the session: its shell and everything it started. */
    end: (id: string) => void;
};

/** What the menu shows for one host's sessions, from the asking pane, under
 *  `notice` (what the last action did) if there is one. */
export function buildHostSessionsMenu(
    conn: string,
    sessions: HostSession[] | string,
    ownBlockId: string,
    actions: HostSessionActions,
    notice?: string
): ContextMenuItem[] {
    if (notice) {
        return [
            { label: notice, enabled: false },
            { type: "separator" },
            ...buildHostSessionsMenu(conn, sessions, ownBlockId, actions),
        ];
    }
    if (typeof sessions === "string") {
        return [{ label: `Could not list sessions on ${conn}: ${sessions}`, enabled: false }];
    }
    if (sessions.length === 0) {
        return [{ label: `No durable sessions on ${conn}`, enabled: false }];
    }
    return sessions.map((s) => {
        const where =
            s.exited != null
                ? `ended (exit ${s.exited})`
                : s.blockid === ownBlockId
                  ? "this pane"
                  : s.blockid
                    ? "another pane"
                    : "no pane";
        const label = `${s.id.slice(0, 12)} · ${formatBytes(s.bytes)} · ${where}`;
        if (s.exited != null) {
            return { label, enabled: false };
        }
        const submenu: ContextMenuItem[] = [];
        if (!s.blockid) {
            submenu.push({ label: "Open in a New Pane", click: () => actions.open(s.id) });
        }
        submenu.push({ label: "End Session (stops its shell)", click: () => actions.end(s.id) });
        return { label, submenu };
    });
}

/** List `conn`'s sessions and show them in a menu at the pane `blockId`,
 *  under `notice` if given. */
export async function showHostSessions(conn: string, blockId: string, notice?: string): Promise<void> {
    let sessions: HostSession[] | string;
    try {
        sessions = await RpcApi.ConnSessionsCommand(
            TabRpcClient,
            { connname: conn, blockid: blockId },
            // Over ssh, and ssh may ask the user something first (srv allows
            // 150 s for both).
            { timeout: 180_000 }
        );
    } catch (e) {
        sessions = String((e as Error)?.message ?? e);
    }
    const menu = buildHostSessionsMenu(conn, sessions, blockId, {
        open: (id) => {
            void createBlock({
                meta: {
                    view: "term",
                    controller: "shell",
                    connection: conn,
                    "term:durable": true,
                    "remote:session_id": id,
                },
            });
        },
        end: (id) => {
            // srv asks the user to confirm in its own window first; then the
            // list again, saying what happened.
            void RpcApi.ConnSessionEndCommand(
                TabRpcClient,
                { connname: conn, sessionid: id, blockid: blockId },
                { timeout: 300_000 }
            )
                .then((ended) => (ended ? `Ended ${id}` : `${id} was already gone`))
                .catch((e) => `Did not end ${id}: ${String((e as Error)?.message ?? e)}`)
                .then((outcome) => showHostSessions(conn, blockId, outcome));
        },
    }, notice);
    // The list arrives after the menu that asked for it has closed: show it
    // over the pane itself.
    const rect = document.querySelector(`[data-blockid="${CSS.escape(blockId)}"]`)?.getBoundingClientRect();
    ContextMenuModel.showContextMenu(menu, {
        clientX: (rect?.left ?? 0) + 24,
        clientY: (rect?.top ?? 0) + 32,
        stopPropagation: () => {},
    } as unknown as MouseEvent);
}
