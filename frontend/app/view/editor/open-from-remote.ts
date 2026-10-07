// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The editor's "Open from remote…" (SPEC_REMOTES_PANE_2026_10_05.md §4.7): the
// connection picker's host list, then a path on the chosen host. The editor
// reads files on SSH hosts only (editor-model's connection()), so WSL is left
// out.

import { remoteSuggestionScopes, type ConnColorOf } from "@/app/modals/conn-remote-items";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { openInPane } from "@/app/view/files/files-open";
import { isSshConnection } from "@/app/view/term/ssh-connection";

/** The hosts to offer, in the picker's sections; a typed `user@host` that
 *  isn't one of them is offered too, as the picker does. */
export function editorHostScopes(
    records: RemoteRecord[],
    typed: string,
    current: string | undefined,
    colorOf: ConnColorOf
): SuggestionConnectionScope[] {
    const scopes = remoteSuggestionScopes(
        records.filter((r) => r.kind !== "wsl"),
        typed,
        current,
        colorOf
    );
    const name = typed.trim();
    const listed = scopes.some((s) => s.items.some((i) => i.value === name));
    if (isSshConnection(name) && !listed) {
        scopes.push({
            headerText: "New host",
            items: [
                {
                    status: "connected",
                    icon: "plus",
                    iconColor: "var(--grey-text-color)",
                    label: name,
                    value: name,
                },
            ],
        });
    }
    return scopes;
}

export interface RemoteOpenTarget {
    blockId: string;
    connection(): string | undefined;
    openFile(path: string): Promise<void>;
}

/** Opens `path` on `host`: in this editor when it is already on that host,
 *  otherwise in a new editor on the host, beside this one. */
export async function openRemoteFile(editor: RemoteOpenTarget, host: string, path: string): Promise<void> {
    const file = path.trim();
    if (!file) throw new Error("Type the path of a file to open");
    if (editor.connection() === host) return editor.openFile(file);
    await openInPane("editor", file, editor.blockId, host);
}
