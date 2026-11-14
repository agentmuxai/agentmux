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

/** A pull request's line (atlas-pr.ts) is bright this long, then stays dimmed until the pull request ends. */
export const PR_FRESH_MS = 5 * 60_000;
/** A pull request's line with no end goes after this long: the fallback for a merge this machine never heard of. */
export const PR_SHOWN_MS = 2 * 60 * 60_000;
export const MAX_PR_LINES = 12;

/**
 * How a line reads: a message that arrived, one kept or queued for later, one refused; a task handed over; or a
 * pull request's review or checks that passed, failed, or said neither.
 */
export type FlowTone = "delivered" | "waiting" | "refused" | "task" | "passed" | "failed" | "neutral";

/**
 * One end of a line: an agent as srv named it (`local`: one of this instance's own, as a pull request's agent
 * always is), or a node of the map by its key (the review bot, CI).
 */
export type FlowEnd = (MessageFlowEnd & { local?: boolean }) | { node: string };

/** One line on show, between two nodes as srv named them. */
export interface AtlasFlow {
    /** Unique to this drawing: a repeat between the same two agents is a new one, so its line draws again. */
    id: string;
    kind: "message" | "task" | "review" | "checks";
    from: FlowEnd;
    to: FlowEnd;
    tone: FlowTone;
    /** A task's number, as "#12", or a pull request's. */
    label: string | null;
    /** When it happened, in ms since 1970: its time on show runs from here. */
    atMs: number;
    /** In its last second: fading out. */
    leaving: boolean;
    /** The pull request a review's or a checks' line is about: the newest of its kind replaces the one before. */
    pr?: string;
    /** What srv called the review or the checks: "approved", "failing". */
    state?: string;
    /** A pull request's line past its first minutes: still true, no longer news. */
    dim?: boolean;
}

/** Whether a line is a pull request's: a review of it, or its checks. */
export const isPrFlow = (flow: Pick<AtlasFlow, "kind">) => flow.kind === "review" || flow.kind === "checks";

const lower = (s: string | undefined) => (s ?? "").trim().toLowerCase();
const endKey = (end: FlowEnd) => ("node" in end ? `node:${end.node}` : `${lower(end.agent)}@${lower(end.host)}`);
const sameEnd = (a: FlowEnd, b: FlowEnd) => endKey(a) === endKey(b);

/** Two drawings of the same line: the newer replaces the older. */
const pairKey = (flow: Pick<AtlasFlow, "kind" | "from" | "to" | "pr">) =>
    flow.pr ? `${flow.kind}|${flow.pr}` : `${flow.kind}|${endKey(flow.from)}|${endKey(flow.to)}`;

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

const SHOWN_MS: Record<AtlasFlow["kind"], number> = {
    message: MESSAGE_SHOWN_MS,
    task: TASK_SHOWN_MS,
    review: PR_SHOWN_MS,
    checks: PR_SHOWN_MS,
};
const MAX_SHOWN: Record<AtlasFlow["kind"], number> = {
    message: MAX_MESSAGES,
    task: MAX_TASKS,
    review: MAX_PR_LINES,
    checks: MAX_PR_LINES,
};
const shownMs = (flow: AtlasFlow) => SHOWN_MS[flow.kind];

/**
 * `flows` at `now`: the ones whose time is up are gone, the ones in their last second are leaving, and a pull
 * request's line past its first minutes is dim. The same list is returned when nothing changed, so a tick that
 * changes nothing redraws nothing.
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
        const dim = isPrFlow(flow) && age >= PR_FRESH_MS;
        const same = leaving === flow.leaving && dim === !!flow.dim;
        if (!same) changed = true;
        out.push(same ? flow : { ...flow, leaving, dim: dim || undefined });
    }
    return changed ? out : flows;
}

/** `flows` without the lines of the pull request `pr`, which has ended; the same list when it has none. */
export function withoutPr(flows: readonly AtlasFlow[], pr: string): readonly AtlasFlow[] {
    return flows.some((f) => f.pr === pr) ? flows.filter((f) => f.pr !== pr) : flows;
}

/**
 * `flows` with `flow` added: it replaces an older drawing between the same two agents, or of the same pull request
 * (a line already drawn is not drawn twice), one srv reported before is left alone, and the oldest of its kind go
 * once there are too many.
 */
export function withFlow(flows: readonly AtlasFlow[], flow: AtlasFlow, now: number): readonly AtlasFlow[] {
    if (now - flow.atMs >= shownMs(flow) || flows.some((f) => f.id === flow.id)) return flows;
    const pair = pairKey(flow);
    const older = flows.find((f) => pairKey(f) === pair);
    if (older && older.atMs > flow.atMs) return flows;
    const kept = flows.filter((f) => f !== older);
    const max = MAX_SHOWN[flow.kind];
    const ofKind = kept.filter((f) => f.kind === flow.kind);
    const dropped = new Set(ofKind.slice(0, Math.max(0, ofKind.length + 1 - max)));
    return [...kept.filter((f) => !dropped.has(f)), flow];
}

/**
 * Finds the node an end of a line names, among the agents drawn. The end's `agent` is the name messages reach it
 * by; its `host`, when srv knew it, says which computer. An agent of that name on that computer is the one; with
 * two there (two channels of one computer), this instance's own. With no host, the only agent of that name
 * anywhere. An end it cannot find, or cannot tell from another, is null: no line, never a wrong one. An end marked
 * `local` is this instance's own agent of that name and no other; an end that is a node of the map is that node.
 */
export function flowEndResolver(
    shown: readonly AtlasAgent[],
    machines: ReadonlyMap<string, AtlasMachine>
): (end: FlowEnd) => string | null {
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
        if ("node" in end) return end.node;
        const named = byRoute.get(lower(end.agent)) ?? [];
        if (end.local) return one(named.filter((a) => !a.remote));
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
/** Half the height of the label's own glyphs: what it needs with no room to spare, in the gap between two rows. */
const LABEL_TIGHT_H = 7;
/** Where along a line its label is tried: the middle first, then further out on either side. */
const LABEL_STOPS = [0.5, 0.45, 0.55, 0.4, 0.6, 0.35, 0.65, 0.3, 0.7, 0.25, 0.75, 0.2, 0.8, 0.15, 0.85];

/** A label already on the map: its point, and half the width of its text. */
export interface PlacedLabel extends Placed {
    halfW: number;
}

/** Half the width of a label's text. A short number is narrow, so two of them fit side by side. */
export const labelHalfWidth = (text: string) => text.length * 3.25 + 1;

/** How far apart along a node's edge two labels are tried: the widest label, and a little air. */
const LABEL_STEP = LABEL_HALF_W * 2 + 4;

/** Whether a label of half-width `halfW` at `p` would lie on the label `taken`. */
function onLabel(taken: PlacedLabel, p: Placed, halfW: number): boolean {
    return Math.abs(p.x - taken.x) < taken.halfW + halfW && Math.abs(p.y - taken.y) < LABEL_TIGHT_H * 2;
}

/** `p` moved sideways to sit next to the label it lies on, on the side it is already nearer; `p` when it lies on none. */
function nextToLabel(taken: readonly PlacedLabel[], p: Placed, halfW: number): Placed {
    const on = taken.find((label) => onLabel(label, p, halfW));
    if (!on) return p;
    const side = p.x >= on.x ? 1 : -1;
    return { x: round(on.x + side * (on.halfW + halfW + 1)), y: p.y };
}

/**
 * Whether a label at `p` would lie on `node`: lines run behind the nodes, so it would be hidden there. A label is
 * taken to be a task number's width at least, and its own (`halfW` each side) when it is longer.
 */
function hidesLabel(node: Placed, p: Placed, halfH = LABEL_HALF_H, halfW = LABEL_HALF_W): boolean {
    const half = Math.max(LABEL_HALF_W, halfW);
    return (
        p.x > node.x - half &&
        p.x < node.x + NODE_W + half &&
        p.y > node.y - halfH &&
        p.y < node.y + NODE_H + halfH
    );
}

/**
 * Where a label goes when no stretch of its line is clear (agents side by side, the line passing behind the ones
 * between): just under or just over the node the line lands on, at the point it lands, on the side it comes from
 * first. When both are taken (by a node, or by the label of another line that lands there), the same two a label's
 * width further along the node's edge, as far as the node goes. Null when none is free.
 */
function labelBesideEnd(to: Placed, end: Placed, fromBelow: boolean, free: (p: Placed) => boolean): Placed | null {
    const under = to.y + NODE_H + LABEL_TIGHT_H;
    const over = to.y - LABEL_TIGHT_H;
    const inward = end.x >= to.x + NODE_W / 2 ? -1 : 1;
    for (let x = end.x; x > to.x - 1 && x < to.x + NODE_W + 1; x += inward * LABEL_STEP) {
        const spot = (fromBelow ? [under, over] : [over, under]).map((y) => ({ x, y })).find(free);
        if (spot) return spot;
    }
    return null;
}

/**
 * The curve from one node's edge to another's, bowed a little to the left of its direction, so a line and its
 * answer do not lie on each other. Its label goes on the stretch nearest its middle that no node of `nodes` covers;
 * when every stretch is covered, beside the node it lands on (`labelBesideEnd`), and failing that on its middle.
 * It also keeps clear of the labels in `taken`, as a label `halfW` wide each side of its point: on a clear stretch
 * that another label has (a line and its answer, close together), it goes next to that label.
 * Null for two nodes on top of each other.
 */
export function flowCurve(
    from: Placed,
    to: Placed,
    nodes: readonly Placed[] = [],
    taken: readonly PlacedLabel[] = [],
    halfW = LABEL_HALF_W
): FlowCurve | null {
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
    const end = { x: round(x2), y: round(y2) };
    const free = (halfH: number) => (p: Placed) =>
        !nodes.some((node) => hidesLabel(node, p, halfH, halfW)) && !taken.some((label) => onLabel(label, p, halfW));
    const clear = LABEL_STOPS.map((t) => nextToLabel(taken, at(t), halfW)).find(free(LABEL_HALF_H));
    return {
        d: `M${round(x1)},${round(y1)} Q${round(cx)},${round(cy)} ${round(x2)},${round(y2)}`,
        end,
        label: clear ?? labelBesideEnd(to, end, cy >= y2, free(LABEL_TIGHT_H)) ?? at(0.5),
    };
}

/** A line to draw between two placed nodes, with the label it carries, if any. */
export interface FlowLine {
    id: string;
    from: Placed;
    to: Placed;
    label: string | null;
}

/**
 * The curve of each line, by its id. A line's label keeps clear of the labels of the lines before it, so two tasks
 * handed to one agent both show their number.
 */
export function flowCurves(lines: readonly FlowLine[], nodes: readonly Placed[]): Map<string, FlowCurve> {
    const taken: PlacedLabel[] = [];
    const curves = new Map<string, FlowCurve>();
    for (const line of lines) {
        const halfW = line.label ? labelHalfWidth(line.label) : LABEL_HALF_W;
        const curve = flowCurve(line.from, line.to, nodes, line.label ? taken : [], halfW);
        if (!curve) continue;
        curves.set(line.id, curve);
        if (line.label) taken.push({ ...curve.label, halfW });
    }
    return curves;
}
