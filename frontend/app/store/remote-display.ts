// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How a pane names and colours the remote it is on
// (SPEC_REMOTES_PANE_2026_10_05.md §4.8): the nickname and colour set in
// Remotes (`connections.<name>` in settings), read straight from the config.

/** `#rrggbb` only; anything else is no colour. */
function validColor(c: unknown): string | undefined {
    return typeof c === "string" && /^#[0-9a-fA-F]{6}$/.test(c) ? c : undefined;
}

export interface RemoteDisplay {
    /** The nickname, else the connection name. */
    name: string;
    color?: string;
}

/** `null` for this computer. */
export function remoteDisplay(
    connections: Record<string, ConnKeywords> | undefined,
    connection: string | null | undefined
): RemoteDisplay | null {
    const conn = connection?.trim();
    if (!conn || conn === "local") return null;
    const keywords = connections?.[conn];
    const nick = keywords?.["display:name"]?.trim();
    return { name: nick || conn, color: validColor(keywords?.["display:color"]) };
}
