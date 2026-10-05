// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The connection picker's remote sections: the Remotes pane's list in a
// smaller form (SPEC_REMOTES_PANE_2026_10_05.md §4.7): Pinned, SSH hosts,
// Recent and WSL, each remote with its nickname and colour. Hidden remotes
// stay out, unless the pane is on one.

import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { displayName, groupRemotes, platformLabel, remoteColor } from "@/app/view/remotes/remotes-sections";

/** The `--conn-icon-color-N` a connected remote's icon takes. */
export type ConnColorOf = (name: string) => number;

function itemOf(r: RemoteRecord, current: string | undefined, colorOf: ConnColorOf): SuggestionConnectionItem {
    const connected = r.status.state === "connected";
    const label = displayName(r);
    return {
        status: "connected",
        icon: r.kind === "wsl" ? "brands@linux" : "server",
        iconColor: connected ? `var(--conn-icon-color-${colorOf(r.name)})` : "var(--grey-text-color)",
        value: r.name,
        label,
        detail: label !== r.name ? r.name : platformLabel(r.platform),
        swatchColor: remoteColor(r),
        current: r.name === current,
    };
}

export function remoteSuggestionScopes(
    records: RemoteRecord[],
    typed: string,
    current: string | undefined,
    colorOf: ConnColorOf
): SuggestionConnectionScope[] {
    return groupRemotes(records, typed)
        .map((g) => ({
            ...g,
            // A hidden remote shows when the pane is on it, or when its name
            // is typed in full: typing a host's name always reaches it.
            records:
                g.section === "hidden"
                    ? g.records.filter((r) => r.name === current || r.name === typed.trim())
                    : g.records,
        }))
        .filter((g) => g.records.length > 0)
        .map((g) => ({
            headerText: g.label,
            items: g.records.map((r) => itemOf(r, current, colorOf)),
        }));
}
