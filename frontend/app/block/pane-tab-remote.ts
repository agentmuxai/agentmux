// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How a pane tab on a remote machine names it: its pill says which remote,
 * with the remote's icon, rather than a chip beside the tab strip that showed
 * only while that tab was active. Used by the tab descriptors of the views
 * that run on a connection (terminal, sysinfo).
 */

import type { PaneTabIcon } from "@/element/pane-tab-model";
import { atoms, getConnStatusAtom } from "@/app/store/global";
import { remoteDisplay } from "@/app/store/remote-display";

/** The remote a block's meta names, or null for this computer. */
export function remoteOf(meta: Record<string, unknown> | undefined): string | null {
    const conn = (meta?.connection as string | undefined)?.trim();
    return conn && conn !== "local" ? conn : null;
}

/** A remote's name for a pill: its Remotes nickname, else the connection. */
export function remoteTabName(connection: string): string {
    return remoteDisplay(atoms.fullConfigAtom()?.connections, connection)?.name ?? connection;
}

/** How many connection colours the theme defines (`--conn-icon-color-N`). */
const CONN_COLORS = 8;

/**
 * A remote's icon for a pill, as the Remotes list and the connection picker
 * draw a remote (conn-remote-items.ts): a server, or Linux for WSL; the
 * remote's own colour, else its connection colour, while connected; grey
 * while not.
 */
export function remoteTabIcon(connection: string): Extract<PaneTabIcon, { kind: "fa" }> {
    const name = connection.startsWith("wsl://") ? "brands@linux" : "server";
    const status = getConnStatusAtom(connection)();
    if (!status?.connected) return { kind: "fa", name, color: "var(--grey-text-color)" };
    const own = remoteDisplay(atoms.fullConfigAtom()?.connections, connection)?.color;
    // By the connection's active number, as the header's chip did.
    const n = (status.activeconnnum ?? 1) % CONN_COLORS || CONN_COLORS;
    return { kind: "fa", name, color: own ?? `var(--conn-icon-color-${n})` };
}
