// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The lines of the Atlas map that are not "reports to": a message from one agent to another, drawn from the
 * sender to the receiver as it happens and gone a few seconds later, and a task one agent handed another, dotted
 * and numbered, for a few minutes (atlas-flow.ts). A message with one end on the map only (its other agent is
 * stopped, filtered out, or not an agent of any machine listed) is a ring round the end that is. A review of a
 * pull request, and its checks, are dashed lines from the review bot's node and CI's to the pull request's agent
 * (atlas-pr.ts), with its number: bright at first, then dim for as long as the pull request is open.
 */

import { createMemo, For, Show, type JSX } from "solid-js";
import { flowCurves, type AtlasFlow, type FlowCurve, type FlowEnd, type FlowLine, type FlowNode } from "./atlas-flow";
import { CONTAINER_FRAME_OUT, NODE_H, NODE_W, type Placed } from "./atlas-layout";

/** How far outside a node its ring is drawn. */
const RING_GAP = 3;
const NO_FRAMES: ReadonlySet<string> = new Set();

type Mark = { curve: FlowCurve; ring: null } | { curve: null; ring: Placed };

/**
 * What to draw for each line, by its id: a curve between its two nodes, or a ring for a message with one end on
 * the map. The curves are worked out together, so that their labels keep clear of each other, of the frame
 * round each node of `framed`, and of the boxes of `avoid`.
 */
function flowMarks(
    flows: readonly AtlasFlow[],
    resolve: (end: FlowEnd) => string | null,
    nodes: ReadonlyMap<string, Placed>,
    framed: ReadonlySet<string>,
    avoid: readonly FlowNode[]
): Map<string, Mark> {
    const node = (key: string): FlowNode | undefined => {
        const p = nodes.get(key);
        return p && framed.has(key) ? { ...p, pad: CONTAINER_FRAME_OUT } : p;
    };
    const at = (end: FlowEnd) => {
        const key = resolve(end);
        return key ? node(key) : undefined;
    };
    const marks = new Map<string, Mark>();
    const lines: FlowLine[] = [];
    for (const flow of flows) {
        const from = at(flow.from);
        const to = at(flow.to);
        const one = to ?? from;
        if (from && to) lines.push({ id: flow.id, from, to, label: flow.label });
        else if (one && flow.kind === "message") marks.set(flow.id, { curve: null, ring: one });
    }
    const all = [...[...nodes.keys()].flatMap((key) => node(key) ?? []), ...avoid];
    for (const [id, curve] of flowCurves(lines, all)) marks.set(id, { curve, ring: null });
    return marks;
}

function FlowMark(props: { flow: AtlasFlow; mark: Mark | undefined }): JSX.Element {
    return (
        <Show when={props.mark}>
            {(m) => (
                <g
                    class={`atlas-flow atlas-flow--${props.flow.tone}`}
                    classList={{ "atlas-flow--leaving": props.flow.leaving, "atlas-flow--dim": !!props.flow.dim }}
                    data-testid="atlas-flow"
                    data-kind={props.flow.kind}
                >
                    <Show when={m().curve}>
                        {(curve) => (
                            <>
                                <path class="atlas-flow__line" d={curve().d} pathLength={props.flow.kind === "message" ? 1 : undefined} />
                                <circle class="atlas-flow__end" cx={curve().end.x} cy={curve().end.y} r="3.5" />
                                <Show when={props.flow.label}>
                                    <text class="atlas-flow__label" x={curve().label.x} y={curve().label.y}>
                                        {props.flow.label}
                                    </text>
                                </Show>
                            </>
                        )}
                    </Show>
                    <Show when={m().ring}>
                        {(ring) => (
                            <rect
                                class="atlas-flow__ping"
                                x={ring().x - RING_GAP}
                                y={ring().y - RING_GAP}
                                width={NODE_W + RING_GAP * 2}
                                height={NODE_H + RING_GAP * 2}
                                rx="6"
                            />
                        )}
                    </Show>
                </g>
            )}
        </Show>
    );
}

export function AtlasFlowLayer(props: {
    flows: readonly AtlasFlow[];
    /** The node an end of a line names, by its key, or null (atlas-flow.ts `flowEndResolver`). */
    resolve: (end: FlowEnd) => string | null;
    nodes: ReadonlyMap<string, Placed>;
    /** The nodes with a frame round them (container agents), by key: a label keeps clear of the frame too. */
    framed?: ReadonlySet<string>;
    /** What else a label keeps clear of (atlas-flow.ts `labelKeepOut`). */
    avoid?: readonly FlowNode[];
    width: number;
    height: number;
}): JSX.Element {
    // By id, so a line keeps its element (and is not drawn again) when only its state changes.
    const ids = createMemo(() => props.flows.map((f) => f.id));
    const byId = createMemo(() => new Map(props.flows.map((f) => [f.id, f] as const)));
    const marks = createMemo(() => flowMarks(props.flows, props.resolve, props.nodes, props.framed ?? NO_FRAMES, props.avoid ?? []));
    return (
        <svg class="atlas-flows" width={props.width} height={props.height} aria-hidden="true">
            <For each={ids()}>
                {(id) => (
                    <Show when={byId().get(id)}>
                        {(flow) => <FlowMark flow={flow()} mark={marks().get(id)} />}
                    </Show>
                )}
            </For>
        </svg>
    );
}
