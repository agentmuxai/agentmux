// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What moves between agents on the Atlas map (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §5.3): a message from
 * one agent to another, and a task one agent gave another. srv says so with the `message-flow` and `task-flow`
 * events, as ids, hosts and times only; this file turns one into a line to draw, finds the node each end names, and
 * keeps the short list of lines on show. It is pure: `atlas-flow-store.ts` listens and keeps time.
 */

import type { MessageFlowEnd } from "@/types/rpc/MessageFlowEnd";
import type { MessageFlowEvent } from "@/types/rpc/MessageFlowEvent";
import type { TaskFlowEvent } from "@/types/rpc/TaskFlowEvent";
import type { AtlasAgent, AtlasMachine } from "./atlas-graph";
import { machineHost } from "./atlas-graph";
import { NODE_H, NODE_W, type Placed } from "./atlas-layout";

/** How long a message's line stays after it is drawn, and a task's. */
export const MESSAGE_SHOWN_MS = 12_000;
export const TASK_SHOWN_MS = 5 * 60_000;
/** A line spends this long, at the end of its time, fading out. */
export const LEAVING_MS = 1_000;
/** The most lines on show: the oldest go first. */
export const MAX_MESSAGES = 40;
export const MAX_TASKS = 12;

/** How a line reads: a message that arrived, one kept or queued for later, one refused; or a task handed over. */
export type FlowTone = "delivered" | "waiting" | "refused" | "task";

/** One line on show, between two agents as srv named them. */
export interface AtlasFlow {
    /** Unique to this drawing: a repeat between the same two agents is a new one, so its line draws again. */
    id: string;
    kind: "message" | "task";
    from: MessageFlowEnd;
    to: MessageFlowEnd;
    tone: FlowTone;
    /** A task's number, as "#12". */
    label: string | null;
    /** When it happened, in ms since 1970: its time on show runs from here. */
    atMs: number;
    /** In its last second: fading out. */
    leaving: boolean;
}

const lower = (s: string | undefined) => (s ?? "").trim().toLowerCase();
const endKey = (end: MessageFlowEnd) => `${lower(end.agent)}@${lower(end.host)}`;
const sameEnd = (a: MessageFlowEnd, b: MessageFlowEnd) => endKey(a) === endKey(b);

/** Two drawings of the same line: the newer replaces the older. */
const pairKey = (flow: Pick<AtlasFlow, "kind" | "from" | "to">) => `${flow.kind}|${endKey(flow.from)}|${endKey(flow.to)}`;

const TONES: Record<MessageFlowEvent["outcome"], FlowTone> = {
    delivered: "delivered",
    held: "waiting",
    queued: "waiting",
    refused: "refused",
};

/** The line for a message, or null for one an agent sent itself. */
export function messageFlow(event: MessageFlowEvent): AtlasFlow | null {
    if (!event.from?.agent || !event.to?.agent || sameEnd(event.from, event.to)) return null;
    return {
        id: `message:${event.msg_id}:${event.at_ms}:${event.outcome}`,
        kind: "message",
        from: event.from,
        to: event.to,
        tone: TONES[event.outcome] ?? "delivered",
        label: null,
        atMs: event.at_ms,
        leaving: false,
    };
}

/**
 * The line for a task one agent filed for, moved to or started on another. Null when it has one end only: the
 * operator did it, it is for anyone, the agent gave it to itself, or it was handed back to nobody.
 */
export function taskFlow(event: TaskFlowEvent): AtlasFlow | null {
    if (!event.by?.agent || !event.to?.agent || sameEnd(event.by, event.to)) return null;
    return {
        id: `task:${event.task.host}:${event.task.board}:${event.task.number}:${event.at_ms}:${event.kind}`,
        kind: "task",
        from: event.by,
        to: event.to,
        tone: "task",
        label: `#${event.task.number}`,
        atMs: event.at_ms,
        leaving: false,
    };
}

const shownMs = (flow: AtlasFlow) => (flow.kind === "task" ? TASK_SHOWN_MS : MESSAGE_SHOWN_MS);

/**
 * `flows` at `now`: the ones whose time is up are gone, the ones in their last second are leaving. The same list
 * is returned when nothing changed, so a tick that changes nothing redraws nothing.
 */
export function flowsAt(flows: readonly AtlasFlow[], now: number): readonly AtlasFlow[] {
    let changed = false;
    const out: AtlasFlow[] = [];
    for (const flow of flows) {
        const age = now - flow.atMs;
        if (age >= shownMs(flow)) {
            changed = true;
            continue;
        }
        const leaving = age >= shownMs(flow) - LEAVING_MS;
        if (leaving !== flow.leaving) changed = true;
        out.push(leaving === flow.leaving ? flow : { ...flow, leaving });
    }
    return changed ? out : flows;
}

/**
 * `flows` with `flow` added: it replaces an older drawing between the same two agents (a line already drawn is not
 * drawn twice), one srv reported before is left alone, and the oldest of its kind go once there are too many.
 */
export function withFlow(flows: readonly AtlasFlow[], flow: AtlasFlow, now: number): readonly AtlasFlow[] {
    if (now - flow.atMs >= shownMs(flow) || flows.some((f) => f.id === flow.id)) return flows;
    const pair = pairKey(flow);
    const older = flows.find((f) => pairKey(f) === pair);
    if (older && older.atMs > flow.atMs) return flows;
    const kept = flows.filter((f) => f !== older);
    const max = flow.kind === "task" ? MAX_TASKS : MAX_MESSAGES;
    const ofKind = kept.filter((f) => f.kind === flow.kind);
    const dropped = new Set(ofKind.slice(0, Math.max(0, ofKind.length + 1 - max)));
    return [...kept.filter((f) => !dropped.has(f)), flow];
}

/**
 * Finds the node an end of a line names, among the agents drawn. The end's `agent` is the name messages reach it
 * by; its `host`, when srv knew it, says which computer. An agent of that name on that computer is the one; with
 * two there (two channels of one computer), this instance's own. With no host, the only agent of that name
 * anywhere. An end it cannot find, or cannot tell from another, is null: no line, never a wrong one.
 */
export function flowEndResolver(
    shown: readonly AtlasAgent[],
    machines: ReadonlyMap<string, AtlasMachine>
): (end: MessageFlowEnd) => string | null {
    const byRoute = new Map<string, AtlasAgent[]>();
    for (const agent of shown) {
        const route = lower(agent.route ?? agent.name);
        byRoute.set(route, [...(byRoute.get(route) ?? []), agent]);
    }
    const hosts = [...machines.values()];
    const one = (agents: AtlasAgent[]): string | null => {
        if (agents.length === 1) return agents[0].key;
        const own = agents.filter((a) => !a.remote);
        return own.length === 1 ? own[0].key : null;
    };
    return (end) => {
        const named = byRoute.get(lower(end.agent)) ?? [];
        const host = lower(end.host);
        if (!host) return one(named);
        if (hosts.some((m) => machineHost(m) === host)) return one(named.filter((a) => machineHost(machines.get(a.machine)) === host));
        // A host no machine is called: this computer's own, while its name is not known yet.
        return one(named.filter((a) => machines.get(a.machine)?.unnamed));
    };
}

/** The curve of a line between two nodes, where it lands, and where its label goes. */
export interface FlowCurve {
    d: string;
    end: Placed;
    label: Placed;
}

const round = (n: number) => Math.round(n * 10) / 10;

/** Half the room a label (a task's number) takes round its point. */
const LABEL_HALF_W = 20;
const LABEL_HALF_H = 9;
/** Where along a line its label is tried: the middle first, then further out on either side. */
const LABEL_STOPS = [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8];

/** Whether a label at `p` would lie on `node`: lines run behind the nodes, so it would be hidden there. */
function hidesLabel(node: Placed, p: Placed): boolean {
    return (
        p.x > node.x - LABEL_HALF_W &&
        p.x < node.x + NODE_W + LABEL_HALF_W &&
        p.y > node.y - LABEL_HALF_H &&
        p.y < node.y + NODE_H + LABEL_HALF_H
    );
}

/**
 * The curve from one node's edge to another's, bowed a little to the left of its direction, so a line and its
 * answer do not lie on each other. Its label goes on the stretch nearest its middle that no node of `nodes` covers
 * (the middle itself when every stretch is covered). Null for two nodes on top of each other.
 */
export function flowCurve(from: Placed, to: Placed, nodes: readonly Placed[] = []): FlowCurve | null {
    const ax = from.x + NODE_W / 2;
    const ay = from.y + NODE_H / 2;
    const bx = to.x + NODE_W / 2;
    const by = to.y + NODE_H / 2;
    const dx = bx - ax;
    const dy = by - ay;
    const length = Math.hypot(dx, dy);
    // How far along the line a node's own box reaches from its centre.
    const reach = Math.min(dx === 0 ? Infinity : NODE_W / 2 / Math.abs(dx), dy === 0 ? Infinity : NODE_H / 2 / Math.abs(dy));
    if (length === 0 || reach >= 0.5) return null;
    const x1 = ax + dx * reach;
    const y1 = ay + dy * reach;
    const x2 = bx - dx * reach;
    const y2 = by - dy * reach;
    const bow = Math.min(28, length * 0.15);
    const cx = (x1 + x2) / 2 + (dy / length) * bow;
    const cy = (y1 + y2) / 2 - (dx / length) * bow;
    const at = (t: number): Placed => ({
        x: round((1 - t) ** 2 * x1 + 2 * (1 - t) * t * cx + t ** 2 * x2),
        y: round((1 - t) ** 2 * y1 + 2 * (1 - t) * t * cy + t ** 2 * y2),
    });
    const clear = LABEL_STOPS.map(at).find((p) => !nodes.some((node) => hidesLabel(node, p)));
    return {
        d: `M${round(x1)},${round(y1)} Q${round(cx)},${round(cy)} ${round(x2)},${round(y2)}`,
        end: { x: round(x2), y: round(y2) },
        label: clear ?? at(0.5),
    };
}
