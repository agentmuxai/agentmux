// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Atlas pane: the Swarm's agents as a map (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md). Machines are regions,
 * agents are nodes, a line runs from a lead to the agent that reports to it. The wheel zooms, a drag on the
 * background pans, Fit shows it all. A node opens a card; the card's Open and Start are the Swarm row's own actions.
 */

import { seatBadgeText, seatTooltipLine } from "@/app/agent-model/seat-format";
import { tip } from "@/app/element/tip";
import { Button, IconButton, SegmentedControl } from "@/app/element/ui";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { openAgentInActiveTab, startAgent } from "@/app/store/agent-runtime-live";
import { revealBlock } from "@/app/util/reveal-block";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { SwarmFilterBar } from "../swarm/swarm-filter-bar";
import type { AtlasAgent, AtlasRegion, AtlasState } from "./atlas-graph";
import {
    FOLD_CHIP,
    layoutAtlas,
    layoutBySeat,
    NODE_H,
    NODE_W,
    type AtlasLayout,
    type RegionBox as RegionBoxLayout,
} from "./atlas-layout";
import type { AtlasData } from "./atlas-model";
import { ATLAS_LAYOUT_META, readAtlasLayout, type AtlasLayoutMode } from "./atlas-layout-mode";
import { useAtlasPanZoom, type AtlasPanZoom } from "./atlas-pan-zoom";
import { clampZoom, type Viewport } from "./atlas-viewport";
import "./atlas.scss";

const STATE_LABELS: Record<AtlasState, string> = {
    working: "Working",
    waiting: "Waiting for an answer",
    idle: "Idle",
    error: "Exited with an error",
    limited: "Out of quota",
    signed_out: "Signed out",
    stopped: "Stopped",
};

const LAYOUT_OPTIONS: { value: AtlasLayoutMode; label: string; title: string }[] = [
    { value: "machine", label: "By machine", title: "Each computer is a region, its agents inside it" },
    { value: "seat", label: "By seat", title: "Every agent in one picture, under whom it reports to" },
];

/** "12k" for a context size. */
function tokensText(n: number): string {
    return n >= 1000 ? `${Math.round(n / 100) / 10}k`.replace(".0k", "k") : String(n);
}

function Node(props: {
    agent: AtlasAgent;
    x: number;
    y: number;
    tint: number | null;
    selected: boolean;
    onSelect: (key: string) => void;
}): JSX.Element {
    const a = () => props.agent;
    return (
        <div
            role="button"
            tabindex="0"
            class={`atlas-node atlas-node--${a().state}`}
            classList={{ "atlas-node--selected": props.selected, "atlas-node--stale": a().stale }}
            data-tint={props.tint ?? undefined}
            data-testid="atlas-node"
            style={{ left: `${props.x}px`, top: `${props.y}px`, width: `${NODE_W}px`, height: `${NODE_H}px` }}
            aria-pressed={props.selected}
            aria-label={`${a().name}, ${STATE_LABELS[a().state]}`}
            {...tip(a().line ?? STATE_LABELS[a().state])}
            onClick={() => props.onSelect(a().key)}
            onKeyDown={(e) => {
                if (e.key !== "Enter" && e.key !== " ") return;
                e.preventDefault();
                props.onSelect(a().key);
            }}
        >
            <span class="atlas-node__top">
                <span class="atlas-node__dot" />
                <span class="atlas-node__name">{a().name}</span>
            </span>
            <span class="atlas-node__sub">
                {a().seat ? seatBadgeText(a().seat!) : STATE_LABELS[a().state]}
            </span>
        </div>
    );
}

function FoldedChip(props: {
    region: AtlasRegion;
    x: number;
    y: number;
    open: boolean;
    onToggle: () => void;
}): JSX.Element {
    return (
        <div class="atlas-fold" style={{ left: `${props.x}px`, top: `${props.y}px` }}>
            <Button
                density="compact"
                aria-expanded={props.open}
                style={{ height: `${FOLD_CHIP.h}px` }}
                onClick={props.onToggle}
            >
                {props.region.folded.length} stopped {props.open ? "▾" : "▸"}
            </Button>
            <Show when={props.open}>
                <ul class="atlas-fold__list">
                    <For each={props.region.folded}>
                        {(agent) => (
                            <li>
                                <span class="atlas-fold__name">{agent.name}</span>
                                <Show when={agent.runtimeId}>
                                    <Button
                                        density="compact"
                                        onClick={() => void openAgentInActiveTab({ agent_id: agent.runtimeId!, name: agent.name })}
                                    >
                                        Open
                                    </Button>
                                    <Button
                                        density="compact"
                                        onClick={() => void startAgent({ agent_id: agent.runtimeId!, name: agent.name })}
                                    >
                                        Start
                                    </Button>
                                </Show>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
        </div>
    );
}

function Card(props: { agent: AtlasAgent; machineTitle: string; onClose: () => void }): JSX.Element {
    const a = () => props.agent;
    return (
        <aside class="atlas-card" data-testid="atlas-card" aria-label={`${a().name} details`}>
            <header class="atlas-card__head">
                <span class={`atlas-node__dot atlas-node--${a().state}`} />
                <strong class="atlas-card__name">{a().name}</strong>
                <IconButton icon="xmark" label="Close" density="compact" onClick={props.onClose} />
            </header>
            <dl class="atlas-card__rows">
                <dt>State</dt>
                <dd>
                    {STATE_LABELS[a().state]}
                    <Show when={a().stale}> (last report is old)</Show>
                </dd>
                <Show when={a().line}>
                    <dt>Doing</dt>
                    <dd>{a().line}</dd>
                </Show>
                <Show when={a().currentTool}>
                    <dt>Tool</dt>
                    <dd>{a().currentTool}</dd>
                </Show>
                <Show when={a().seat}>
                    <dt>Seat</dt>
                    <dd>{seatTooltipLine(a().seat!)}</dd>
                </Show>
                <dt>Machine</dt>
                <dd>{props.machineTitle}</dd>
                <Show when={a().provider}>
                    <dt>Provider</dt>
                    <dd>{a().provider}</dd>
                </Show>
                <Show when={a().contextTokens}>
                    <dt>Context</dt>
                    <dd>{tokensText(a().contextTokens!)} tokens</dd>
                </Show>
                <Show when={a().spend}>
                    <dt>Spend today</dt>
                    <dd>
                        {Math.round((a().spend!.spent / a().spend!.cap) * 100)}% of its cap
                    </dd>
                </Show>
            </dl>
            <Show when={!a().remote && (a().blockId || a().runtimeId)}>
                <div class="atlas-card__actions">
                    <Button
                        density="compact"
                        onClick={() => {
                            if (a().blockId) void revealBlock(a().blockId!, { source: "atlas" });
                            else void openAgentInActiveTab({ agent_id: a().runtimeId!, name: a().name });
                        }}
                    >
                        Open
                    </Button>
                </div>
            </Show>
        </aside>
    );
}

/** The path of a line from a lead's node to a report's: down from the lead's bottom, up to the report's top. */
function edgePath(from: { x: number; y: number }, to: { x: number; y: number }): string {
    const x1 = from.x + NODE_W / 2;
    const y1 = from.y + NODE_H;
    const x2 = to.x + NODE_W / 2;
    const y2 = to.y;
    const mid = (y1 + y2) / 2;
    return `M${x1},${y1} C${x1},${mid} ${x2},${mid} ${x2},${y2}`;
}

/** The map itself: a box per machine, the lines, and a node per running agent, in map pixels. */
function AtlasMap(props: {
    data: AtlasData;
    layout: AtlasLayout;
    view: Viewport;
    mode: AtlasLayoutMode;
    selected: string | null;
    onSelect: (key: string) => void;
}): JSX.Element {
    const [openFolds, setOpenFolds] = createSignal<ReadonlySet<string>>(new Set());
    const toggleFold = (key: string) =>
        setOpenFolds((s) => {
            const next = new Set(s);
            if (!next.delete(key)) next.add(key);
            return next;
        });
    const regions = () => props.data.graph().regions;
    const shownAgents = createMemo(() => regions().flatMap((r) => r.shown));
    const machineIndex = (machine: string) => regions().findIndex((r) => r.key === machine);
    return (
        <div
            class="atlas-world"
            style={{
                width: `${props.layout.width}px`,
                height: `${props.layout.height}px`,
                transform: `translate(${props.view.x}px, ${props.view.y}px) scale(${clampZoom(props.view.k)})`,
            }}
        >
            <For each={props.layout.regions}>
                {(box) => (
                    <Show when={regions().find((r) => r.key === box.key)}>
                        {(r) => (
                            <RegionBox box={box} region={r()} open={openFolds().has(r().key)} onToggle={() => toggleFold(r().key)} />
                        )}
                    </Show>
                )}
            </For>
            <svg class="atlas-edges" width={props.layout.width} height={props.layout.height} aria-hidden="true">
                <For each={props.data.graph().edges}>
                    {(edge) => {
                        const from = () => props.layout.nodes.get(edge.from);
                        const to = () => props.layout.nodes.get(edge.to);
                        return (
                            <Show when={from() && to()}>
                                <path
                                    class="atlas-edge"
                                    classList={{ "atlas-edge--spawned": edge.kind === "spawned" }}
                                    d={edgePath(from()!, to()!)}
                                />
                            </Show>
                        );
                    }}
                </For>
            </svg>
            <For each={shownAgents()}>
                {(agent) => (
                    <Show when={props.layout.nodes.get(agent.key)}>
                        {(p) => (
                            <Node
                                agent={agent}
                                x={p().x}
                                y={p().y}
                                tint={props.mode === "seat" ? machineIndex(agent.machine) % 6 : null}
                                selected={props.selected === agent.key}
                                onSelect={props.onSelect}
                            />
                        )}
                    </Show>
                )}
            </For>
        </div>
    );
}

/** One machine box, with its title and badge, and its folded chip. */
function RegionBox(props: { box: RegionBoxLayout; region: AtlasRegion; open: boolean; onToggle: () => void }): JSX.Element {
    const r = () => props.region;
    const box = () => props.box;
    return (
        <>
            <section
                class="atlas-region"
                classList={{ "atlas-region--stale": r().stale }}
                data-testid="atlas-region"
                style={{ left: `${box().x}px`, top: `${box().y}px`, width: `${box().w}px`, height: `${box().h}px` }}
            >
                <header class="atlas-region__head">
                    <span class="atlas-region__title">{r().title}</span>
                    <span class="atlas-region__badge">
                        {r().badge}
                        {r().platform ? ` · ${r().platform}` : ""}
                        {r().stale ? " · stale" : ""}
                    </span>
                </header>
            </section>
            <Show when={r().folded.length > 0}>
                <FoldedChip region={r()} x={box().x + FOLD_CHIP.x} y={box().foldY} open={props.open} onToggle={props.onToggle} />
            </Show>
        </>
    );
}

/** The bar over the map: the Swarm filter, the layout toggle, zoom and fit. */
function AtlasBar(props: { data: AtlasData; ctx: PaneTabHostContext; mode: AtlasLayoutMode; zoom: AtlasPanZoom }): JSX.Element {
    return (
        <div class="atlas-bar">
            <SwarmFilterBar filter={props.data.swarm.filter} />
            <SegmentedControl<AtlasLayoutMode>
                density="compact"
                ariaLabel="Layout"
                options={LAYOUT_OPTIONS}
                value={props.mode}
                onChange={(v) => void props.ctx.setMeta({ [ATLAS_LAYOUT_META]: v === "seat" ? "seat" : null })}
            />
            <IconButton icon="magnifying-glass-minus" label="Zoom out" density="compact" onClick={() => props.zoom.zoomBy(1 / 1.25)} />
            <IconButton icon="magnifying-glass-plus" label="Zoom in" density="compact" onClick={() => props.zoom.zoomBy(1.25)} />
            <IconButton icon="expand" label="Fit all" density="compact" onClick={props.zoom.fit} />
        </div>
    );
}

export function AtlasView(props: { data: AtlasData; ctx: PaneTabHostContext }): JSX.Element {
    const { data, ctx } = props;
    const mode = createMemo<AtlasLayoutMode>(() => readAtlasLayout(ctx.meta()));
    const layout = createMemo<AtlasLayout>(() => (mode() === "seat" ? layoutBySeat(data.graph()) : layoutAtlas(data.graph())));
    const [selected, setSelected] = createSignal<string | null>(null);
    const zoom = useAtlasPanZoom(() => layout(), mode, () => setSelected(null));
    const shownCount = createMemo(() => data.graph().regions.reduce((n, r) => n + r.shown.length, 0));
    const stoppedCount = createMemo(() => data.graph().regions.reduce((n, r) => n + r.folded.length, 0));
    const selectedAgent = () => {
        const key = selected();
        return key ? data.agents().get(key) : undefined;
    };
    return (
        <div class="atlas" data-testid="atlas" onKeyDown={(e) => e.key === "Escape" && setSelected(null)}>
            <AtlasBar data={data} ctx={ctx} mode={mode()} zoom={zoom} />
            <div
                class="atlas-viewport"
                ref={zoom.setViewport}
                data-testid="atlas-viewport"
                onWheel={zoom.onWheel}
                onPointerDown={zoom.onPointerDown}
                onPointerMove={zoom.onPointerMove}
                onPointerUp={zoom.onPointerUp}
                onPointerCancel={zoom.onPointerUp}
            >
                <Show when={shownCount() > 0 || stoppedCount() > 0} fallback={<div class="atlas-empty">No agents to show yet.</div>}>
                    <AtlasMap data={data} layout={layout()} view={zoom.view()} mode={mode()} selected={selected()} onSelect={setSelected} />
                </Show>
                <Show when={mode() === "seat" && stoppedCount() > 0}>
                    <div class="atlas-stopped-count">{stoppedCount()} stopped, not drawn</div>
                </Show>
                <Show when={selectedAgent()}>
                    {(agent) => (
                        <Card
                            agent={agent()}
                            machineTitle={data.machines().get(agent().machine)?.title ?? ""}
                            onClose={() => setSelected(null)}
                        />
                    )}
                </Show>
            </div>
        </div>
    );
}
