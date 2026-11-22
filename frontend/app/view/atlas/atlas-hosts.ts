// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which host a node is on, and the ring it wears for it. The Atlas draws no box per host: each node says its host in
 * words (a label, its tooltip and its accessible name) and carries one more border, outside its own, in a colour that
 * belongs to the host. The colour is never the only cue: with more hosts than colours a ring is drawn dashed or
 * dotted as well, and the label names the host either way.
 *
 * A host's colour is stable: it comes from the host's name (a hash of it), never from the order hosts arrived in, so
 * it is the same after a restart and on every machine that looks at the same fleet. Two hosts that hash to one slot
 * are settled in the order of their names, the later taking the next free slot, so the answer does not depend on the
 * order the machines are listed in. (The set of hosts decides it: when one comes or goes, only a host that shared its
 * slot can move.)
 */

import { machineHost, type AtlasMachine } from "./atlas-graph";

/** The ring colours (atlas.scss `--atlas-host-1` to `-3`): hues no state, line or selection uses. */
export const HOST_COLOURS = 3;
/** How the ring is drawn, in the order slots run out of colours. */
export const HOST_LINES = ["solid", "dashed", "dotted"] as const;
export type HostLine = (typeof HOST_LINES)[number];
/** How many hosts can have a ring of their own. */
export const HOST_SLOTS = HOST_COLOURS * HOST_LINES.length;

export interface HostStyle {
    /** The host's identity: its lowercased computer name, or the machine's key when this computer is not named yet. */
    id: string;
    /** The host's name as it is written. */
    name: string;
    /** Which of `HOST_COLOURS` (0-based). */
    colour: number;
    line: HostLine;
    /** Another host has the same ring (more hosts than slots): the label is what tells them apart. */
    shared: boolean;
}

/** The computer a machine is: "narko" for "narko" and for one of its channels ("narko · dev"). */
export function hostNameOf(machine: Pick<AtlasMachine, "title" | "unnamed">): string {
    return machine.unnamed ? machine.title : machine.title.split("·")[0].trim() || machine.title;
}

/** The host a machine is on, for telling hosts apart: its computer's name, lowercased, else the machine's own key. */
export function hostIdOf(machine: AtlasMachine): string {
    return machineHost(machine) ?? `unnamed:${machine.key}`;
}

/** FNV-1a, 32 bits: the same number for the same text everywhere. */
function hash(text: string): number {
    let h = 0x811c9dc5;
    for (let i = 0; i < text.length; i++) {
        h ^= text.charCodeAt(i);
        h = Math.imul(h, 0x01000193);
    }
    return h >>> 0;
}

/** The ring of each host of `machines`, by the machine's key. */
export function hostStyles(machines: readonly AtlasMachine[]): Map<string, HostStyle> {
    const hosts = new Map<string, string>();
    for (const m of machines) if (!hosts.has(hostIdOf(m))) hosts.set(hostIdOf(m), hostNameOf(m));
    const taken = new Map<number, number>();
    const byHost = new Map<string, HostStyle>();
    for (const id of [...hosts.keys()].sort()) {
        let slot = hash(id) % HOST_SLOTS;
        const room = taken.size < HOST_SLOTS;
        while (room && taken.has(slot)) slot = (slot + 1) % HOST_SLOTS;
        const shared = taken.has(slot);
        taken.set(slot, (taken.get(slot) ?? 0) + 1);
        byHost.set(id, { id, name: hosts.get(id)!, colour: slot % HOST_COLOURS, line: HOST_LINES[Math.floor(slot / HOST_COLOURS)], shared });
    }
    // Two hosts of one slot are both "shared", not only the later one.
    const used = new Map<string, number>();
    for (const s of byHost.values()) used.set(`${s.colour}:${s.line}`, (used.get(`${s.colour}:${s.line}`) ?? 0) + 1);
    for (const s of byHost.values()) s.shared = (used.get(`${s.colour}:${s.line}`) ?? 0) > 1;
    return new Map(machines.map((m) => [m.key, byHost.get(hostIdOf(m))!] as const));
}

/** The distinct hosts of `styles`, by name. */
export function distinctHosts(styles: ReadonlyMap<string, HostStyle>): HostStyle[] {
    const byId = new Map<string, HostStyle>();
    for (const s of styles.values()) byId.set(s.id, s);
    return [...byId.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** Whether rings are drawn: with one host there is nothing to tell apart, and the label alone says where it is. */
export function ringsDrawn(styles: ReadonlyMap<string, HostStyle>): boolean {
    return distinctHosts(styles).length > 1;
}
