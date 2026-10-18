// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What kind of process a row is: its label, its icon and its colour
// (SPEC_TOWER_THEME_SURFACE_AND_COLOR_2026_10_17.md §3). srv decides the kind
// (`backend/tower_kind.rs`); the pane never guesses from a name. The colour
// is a class, `tower-kind--<kind>` (tower-view.scss), which sets
// `--tower-kind-color` for the icon and `--tower-kind-fill` for bars: the five
// coloured kinds take the theme's `--process-kind-*`, the others graded tints
// of the text colour.

import type { TowerProcess, TowerProcessKind } from "@/app/store/rpc-api";
import type { Sort } from "./tower-util";

export interface KindInfo {
    label: string;
    /** On a chip, where room is short. */
    short: string;
    /** Font Awesome icon name, and its style. */
    icon: string;
    regular?: boolean;
    /** One of the five kinds with a colour of its own. */
    coloured: boolean;
}

export const KINDS: Record<TowerProcessKind, KindInfo> = {
    agents: { label: "Agents & AI", short: "Agents", icon: "robot", coloured: true },
    agentmux: { label: "AgentMux", short: "AgentMux", icon: "layer-group", coloured: true },
    shells: { label: "Shells & commands", short: "Shells", icon: "terminal", coloured: true },
    builds: { label: "Builds & tools", short: "Builds", icon: "hammer", coloured: true },
    runtimes: { label: "Runtimes & servers", short: "Runtimes", icon: "server", coloured: true },
    browsers: { label: "Browsers & web views", short: "Browsers", icon: "globe", coloured: false },
    containers: { label: "Containers & VMs", short: "VMs", icon: "cube", coloured: false },
    editors: { label: "Editors & IDEs", short: "Editors", icon: "code", coloured: false },
    apps: { label: "Apps", short: "Apps", icon: "window-maximize", regular: true, coloured: false },
    system: { label: "System", short: "System", icon: "microchip", coloured: false },
};

/** The order kinds are listed in: the work an agent runs or is, then the
 *  rest, the system last. */
export const KIND_ORDER: readonly TowerProcessKind[] = [
    "agents",
    "agentmux",
    "shells",
    "builds",
    "runtimes",
    "browsers",
    "containers",
    "editors",
    "apps",
    "system",
];

export function isKind(v: unknown): v is TowerProcessKind {
    return typeof v === "string" && (KIND_ORDER as readonly string[]).includes(v);
}

/** A row's kind, for grouping: an AgentMux that predates kinds sends none,
 *  and its rows count as apps. */
export function kindOf(p: TowerProcess): TowerProcessKind {
    return p.kind ?? "apps";
}

/** The processes of one kind, with their totals. */
export interface KindGroup {
    kind: TowerProcessKind;
    processes: TowerProcess[];
    cpu?: number;
    mem?: number;
}

function sumKnown(xs: (number | undefined)[]): number | undefined {
    const known = xs.filter((x): x is number => x != null);
    return known.length ? known.reduce((a, b) => a + b, 0) : undefined;
}

/** Every kind present in `processes`, in `KIND_ORDER`. */
export function kindGroups(processes: TowerProcess[]): KindGroup[] {
    const byKind = new Map<TowerProcessKind, TowerProcess[]>();
    for (const p of processes) {
        const k = kindOf(p);
        const list = byKind.get(k);
        if (list) list.push(p);
        else byKind.set(k, [p]);
    }
    return KIND_ORDER.filter((k) => byKind.has(k)).map((kind) => {
        const list = byKind.get(kind)!;
        return {
            kind,
            processes: list,
            cpu: sumKnown(list.map((p) => p.cpu)),
            mem: sumKnown(list.map((p) => p.mem)),
        };
    });
}

/** Kinds by `sort`'s totals; by name means their own order. System is
 *  always last: it is the background. */
export function sortKindGroups(groups: KindGroup[], sort: Sort): KindGroup[] {
    const order = (g: KindGroup) => KIND_ORDER.indexOf(g.kind);
    const value = (g: KindGroup) => (sort.key === "cpu" ? g.cpu : sort.key === "mem" ? g.mem : g.processes.length);
    return [...groups].sort((a, b) => {
        if ((a.kind === "system") !== (b.kind === "system")) return a.kind === "system" ? 1 : -1;
        if (sort.key === "name") return sort.desc ? order(b) - order(a) : order(a) - order(b);
        const c = (value(a) ?? -1) - (value(b) ?? -1);
        return (sort.desc ? -c : c) || order(a) - order(b);
    });
}
