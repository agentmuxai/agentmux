// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Tower pane's title from its meta alone (tower.tsx, `PaneTabManifest.title`). Light, so the manifest can read it
// without loading the view.

/** "Tower", or "Tower · Processes" / "Tower · Panes" for the view the meta picks; "host" is what Processes was called
 *  until 2026-10-10. A pane on another computer is named after its host, which the meta does not hold: undefined then. */
export function towerTitle(meta: MetaType | undefined): string | undefined {
    const connection = meta?.["tower:connection"];
    if (typeof connection === "string" && connection !== "" && connection !== "local") return undefined;
    const view = meta?.["tower:view"];
    if (view === "processes" || view === "host") return "Tower · Processes";
    return view === "panes" ? "Tower · Panes" : "Tower";
}
