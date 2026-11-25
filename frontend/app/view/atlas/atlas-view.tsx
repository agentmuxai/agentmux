// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Atlas pane: the Swarm's agents as a map (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md). Agents are nodes, each
 * saying its host in words and wearing a ring in the host's colour (atlas-hosts.ts); a line runs from a lead to the
 * agent that reports to it. The wheel zooms, a drag on the
 * background pans, Fit shows it all. A node opens a card; the card's Open and Start are the Swarm row's own actions.
 * A box dragged with Shift held (or with Select on) selects the agents it touches, for the Swarm's own Broadcast,
 * Stop and Supervisor mode (atlas-fleet-bar.tsx). A message between two agents draws a line from its sender to its
 * receiver as it happens, and a task one agent hands another a dotted one (atlas-flow-layer.tsx). A container agent
 * has its container as a frame round it, and the review bot and CI are nodes of their own (atlas-extras.tsx).
 */

import { seatTooltipLine } from "@/app/agent-model/seat-format";
import { SeatTag } from "@/app/agent-model/seat-tag";
import { tip } from "@/app/element/tip";
import { Button, CloseButton, IconButton, SegmentedControl } from "@/app/element/ui";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { openAgentInActiveTab, startAgent } from "@/app/store/agent-runtime-live";
import { revealBlock } from "@/app/util/reveal-block";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { ByKey } from "./atlas-by-key";
import { SwarmFilterBar } from "../swarm/swarm-filter-bar";
import { CardFacts } from "./atlas-card-facts";
import { inContainer } from "./atlas-containers";
import { edgePath, edgePoints } from "./atlas-edge-path";
import { ExtraCard, ExtraNodes } from "./atlas-extras";
import { AtlasFleetBar } from "./atlas-fleet-bar";
import { flowEndResolver, labelKeepOut } from "./atlas-flow";
import { AtlasFlowLayer } from "./atlas-flow-layer";
import type { AtlasAgent, AtlasRegion, AtlasState } from "./atlas-graph";
import { AtlasLegend, HostLabel, hostAttrs, machineDetail, onHost, ringVars } from "./atlas-host-mark";
import { hostStyles, ringsDrawn, type HostStyle } from "./atlas-hosts";
import { fitScale, layoutAtlas, layoutBySeat, NODE_H, NODE_W, type AtlasLayout, type NodeHeights } from "./atlas-layout";
import type { AtlasData } from "./atlas-model";
import { ATLAS_LAYOUT_META, readAtlasLayout, type AtlasLayoutMode } from "./atlas-layout-mode";
import { useAtlasLasso } from "./atlas-lasso";
import { NOT_BACKGROUND, useAtlasPanZoom, type AtlasPanZoom } from "./atlas-pan-zoom";
import { selectionKeyOf } from "./atlas-select";
import { clampZoom, type Viewport } from "./atlas-viewport";
import { nodeHeights, nodeTasksOf, showTaskLines } from "./atlas-node-tasks";
import { NodeTasksList, TaskCount } from "./atlas-node-tasks-view";
import type { AtlasWork, NodeTasks } from "./atlas-work";
import { useAtlasTasks, useAtlasWork } from "./atlas-work-store";
import { t } from "@/app/i18n/i18n";
import "./atlas.scss";

// Built when drawn, so a change of language shows.
const stateLabel = (state: AtlasState): string =>
    ({
        working: t("atlas.state.working"),
        waiting: t("atlas.state.waiting"),
        idle: t("atlas.state.idle"),
        error: t("atlas.state.error"),
        limited: t("atlas.state.limited"),
        signed_out: t("atlas.state.signedOut"),
        stopped: t("atlas.state.stopped"),
    })[state];

const layoutOptions = (): { value: AtlasLayoutMode; label: string; title: string }[] => [
    { value: "machine", label: t("atlas.layout.machine"), title: t("atlas.layout.machineTip") },
    { value: "seat", label: t("atlas.layout.seat"), title: t("atlas.layout.seatTip") },
];

/** "12k" for a context size. */
function tokensText(n: number): string {
    return n >= 1000 ? `${Math.round(n / 100) / 10}k`.replace(".0k", "k") : String(n);
}

function Node(props: {
    agent: AtlasAgent;
    x: number;
    y: number;
    /** Its height: `NODE_H`, or taller with the lines of its tasks (atlas-layout.ts `nodeHeight`). */
    h: number;
    /** What it lists of its tasks, and whether the map is big enough to list them line by line. */
    tasks?: NodeTasks;
    lines: boolean;
    /** Its host's ring and name; undefined for a machine this view does not know. */
    host: HostStyle | undefined;
    /** Rings are drawn (more than one host). */
    ring: boolean;
    selected: boolean;
    /** In the selection a fleet action reaches, or inside the box being dragged. */
    picked: boolean;
    onSelect: (key: string, e: MouseEvent | KeyboardEvent) => void;
}): JSX.Element {
    const a = () => props.agent;
    return (
        <div
            role="button"
            tabindex="0"
            class={`atlas-node atlas-node--${a().state}`}
            classList={{
                "atlas-node--selected": props.selected,
                "atlas-node--picked": props.picked,
                "atlas-node--stale": a().stale,
                "atlas-node--container": inContainer(a()),
                "atlas-node--tasks": props.lines && !!props.tasks,
            }}
            {...hostAttrs(props.host, props.ring)}
            data-container={a().container?.state}
            data-testid="atlas-node"
            style={{ left: `${props.x}px`, top: `${props.y}px`, width: `${NODE_W}px`, height: `${props.h}px` }}
            aria-pressed={props.selected}
            aria-label={`${a().name}, ${stateLabel(a().state)}${onHost(props.host)}${inContainer(a()) ? `, ${t("atlas.node.inContainer")}` : ""}${props.picked ? `, ${t("atlas.node.selected")}` : ""}`}
            {...tip(`${a().line ?? stateLabel(a().state)}${onHost(props.host)}`)}
            onClick={(e) => props.onSelect(a().key, e)}
            onKeyDown={(e) => {
                // A key on one of the task links is theirs.
                if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
                e.preventDefault();
                props.onSelect(a().key, e);
            }}
        >
            <span class="atlas-node__head">
                <span class="atlas-node__top">
                    <span class="atlas-node__dot" />
                    <span class="atlas-node__name">{a().name}</span>
                    {/* Where the map is too small to list the tasks, how many there are. */}
                    <Show when={props.lines ? undefined : props.tasks}>{(tasks) => <TaskCount total={tasks().total} />}</Show>
                    <Show when={inContainer(a())}>
                        <i class="fa-solid fa-box atlas-node__kind" aria-hidden="true" />
                    </Show>
                    <HostLabel style={props.host} />
                    <Show when={props.picked}>
                        <i class="fa-solid fa-check atlas-node__check" aria-hidden="true" />
                    </Show>
                </span>
                <span class="atlas-node__sub">
                    <Show when={a().seat} fallback={stateLabel(a().state)}>
                        {(seat) => <SeatTag seat={seat()} size="xs" class="atlas-node__seat" />}
                    </Show>
                </span>
            </span>
            <Show when={props.lines && props.tasks}>{(tasks) => <NodeTasksList tasks={tasks()} />}</Show>
        </div>
    );
}

/** The stopped agents, which the map does not draw: one chip that lists them with the host each is on, to Open or Start. */
function StoppedChip(props: { agents: { agent: AtlasAgent; host: string }[] }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    return (
        <div class="atlas-fold" data-testid="atlas-stopped">
            <Button density="compact" aria-expanded={open()} onClick={() => setOpen(!open())}>
                {t("atlas.stopped", { n: props.agents.length })} {open() ? "▾" : "▴"}
            </Button>
            <Show when={open()}>
                <ul class="atlas-fold__list">
                    <For each={props.agents}>
                        {({ agent, host }) => (
                            <li>
                                <span class="atlas-fold__name">{agent.name}</span>
                                <span class="atlas-fold__host">{host}</span>
                                <Show when={agent.runtimeId}>
                                    <Button
                                        density="compact"
                                        onClick={() => void openAgentInActiveTab({ agent_id: agent.runtimeId!, name: agent.name })}
                                    >
                                        {t("atlas.open")}
                                    </Button>
                                    <Button
                                        density="compact"
                                        onClick={() => void startAgent({ agent_id: agent.runtimeId!, name: agent.name })}
                                    >
                                        {t("atlas.start")}
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

function Card(props: {
    agent: AtlasAgent;
    machineTitle: string;
    /** How the machine was found and what it says of itself (atlas-host-mark.tsx `machineDetail`). */
    machineDetail: string;
    agents: ReadonlyMap<string, AtlasAgent>;
    work: AtlasWork | null;
    onClose: () => void;
}): JSX.Element {
    const a = () => props.agent;
    return (
        <aside class="atlas-card" data-testid="atlas-card" aria-label={t("atlas.card.aria", { name: a().name })}>
            <header class="atlas-card__head">
                <span class={`atlas-node__dot atlas-node--${a().state}`} />
                <strong class="atlas-card__name">{a().name}</strong>
                <CloseButton size="sm" onClick={props.onClose} />
            </header>
            <dl class="atlas-card__rows">
                <dt>{t("atlas.card.state")}</dt>
                <dd>
                    {stateLabel(a().state)}
                    <Show when={a().stale}> {t("atlas.card.stale")}</Show>
                </dd>
                <Show when={a().line}>
                    <dt>{t("atlas.card.doing")}</dt>
                    <dd>{a().line}</dd>
                </Show>
                <Show when={a().currentTool}>
                    <dt>{t("atlas.card.tool")}</dt>
                    <dd>{a().currentTool}</dd>
                </Show>
                <Show when={a().seat}>
                    <dt>{t("atlas.card.seat")}</dt>
                    <dd>{seatTooltipLine(a().seat!)}</dd>
                </Show>
                <dt>{t("atlas.card.host")}</dt>
                <dd>
                    {props.machineTitle}
                    <Show when={props.machineDetail}>{(text) => <span class="atlas-card__detail"> · {text()}</span>}</Show>
                </dd>
                <Show when={a().provider}>
                    <dt>{t("atlas.card.provider")}</dt>
                    <dd>{a().provider}</dd>
                </Show>
                <CardFacts agent={a()} agents={props.agents} work={props.work} />
                <Show when={a().contextTokens}>
                    <dt>{t("atlas.card.context")}</dt>
                    <dd>{t("atlas.card.tokens", { size: tokensText(a().contextTokens!) })}</dd>
                </Show>
                <Show when={a().spend}>
                    <dt>{t("atlas.card.spend")}</dt>
                    <dd>
                        {t("atlas.card.ofCap", { pct: Math.round((a().spend!.spent / a().spend!.cap) * 100) })}
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
                        {t("atlas.open")}
                    </Button>
                </div>
            </Show>
        </aside>
    );
}

const NO_NODE_HEIGHTS: NodeHeights = new Map();

/** The map itself: the lines and a node per running agent, in map pixels. */
function AtlasMap(props: {
    data: AtlasData;
    layout: AtlasLayout;
    view: Viewport;
    hosts: ReadonlyMap<string, HostStyle>;
    /** Rings are drawn (more than one host). */
    ring: boolean;
    selected: string | null;
    picked: (agent: AtlasAgent) => boolean;
    /** What each node lists of its agent's tasks, and whether the map lists them line by line. */
    tasks: ReadonlyMap<string, NodeTasks>;
    lines: boolean;
    onSelect: (key: string, e: MouseEvent | KeyboardEvent) => void;
}): JSX.Element {
    const regions = () => props.data.graph().regions;
    const shownAgents = createMemo(() => regions().flatMap((r) => r.shown));
    const flowEnd = createMemo(() => flowEndResolver(shownAgents(), props.data.machines()));
    const framed = createMemo(() => new Set(shownAgents().filter(inContainer).map((a) => a.key)));
    const keepOut = createMemo(() => labelKeepOut(props.layout));
    // The lines of reports-to, for a task's number to keep clear of.
    const reportLines = createMemo(() =>
        props.data.graph().edges.flatMap((edge) => {
            const from = props.layout.nodes.get(edge.from);
            const to = props.layout.nodes.get(edge.to);
            return from && to ? [edgePoints(from, to, props.layout.wrapped.get(edge.to) === edge.from, props.layout.gaps)] : [];
        })
    );
    return (
        <div
            class="atlas-world"
            style={{
                width: `${props.layout.width}px`,
                height: `${props.layout.height}px`,
                transform: `translate(${props.view.x}px, ${props.view.y}px) scale(${clampZoom(props.view.k)})`,
                // A container's frame closes in with the gaps it stands in (atlas.scss `.atlas-node--container`).
                "--atlas-frame-out": `${props.layout.gaps.frame}px`,
                // So does a host's ring, so two neighbours' rings never meet (atlas-host-mark.tsx `ringVars`).
                ...ringVars(props.layout.gaps),
            }}
        >
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
                                    d={edgePath(from()!, to()!, props.layout.wrapped.get(edge.to) === edge.from, props.layout.gaps)}
                                />
                            </Show>
                        );
                    }}
                </For>
            </svg>
            <AtlasFlowLayer
                flows={props.data.flows()}
                resolve={flowEnd()}
                nodes={props.layout.nodes}
                framed={framed()}
                avoid={keepOut()}
                frameOut={props.layout.gaps.frame}
                edges={reportLines()}
                width={props.layout.width}
                height={props.layout.height}
            />
            <ByKey each={shownAgents()}>
                {(agent, key) => (
                    <Show when={props.layout.nodes.get(key)}>
                        {(p) => (
                            <Node
                                agent={agent()}
                                x={p().x}
                                y={p().y}
                                h={p().h ?? NODE_H}
                                tasks={props.tasks.get(key)}
                                lines={props.lines}
                                host={props.hosts.get(agent().machine)}
                                ring={props.ring}
                                selected={props.selected === key}
                                picked={props.picked(agent())}
                                onSelect={props.onSelect}
                            />
                        )}
                    </Show>
                )}
            </ByKey>
            <ExtraNodes
                graph={props.data.graph()}
                nodes={props.layout.nodes}
                hosts={props.hosts}
                ring={props.ring}
                selected={props.selected}
                onSelect={props.onSelect}
            />
        </div>
    );
}

/** The bar over the map: the Swarm filter, the layout toggle, Select, zoom and fit. */
function AtlasBar(props: {
    data: AtlasData;
    ctx: PaneTabHostContext;
    mode: AtlasLayoutMode;
    zoom: AtlasPanZoom;
    selecting: boolean;
    onSelecting: (on: boolean) => void;
}): JSX.Element {
    return (
        <div class="atlas-bar">
            <SwarmFilterBar filter={props.data.swarm.filter} />
            <SegmentedControl<AtlasLayoutMode>
                density="compact"
                ariaLabel={t("atlas.layout.aria")}
                options={layoutOptions()}
                value={props.mode}
                onChange={(v) => void props.ctx.setMeta({ [ATLAS_LAYOUT_META]: v === "seat" ? "seat" : null })}
            />
            <IconButton
                icon="object-group"
                label={props.selecting ? t("atlas.select.on") : t("atlas.select.off")}
                density="compact"
                pressed={props.selecting}
                onClick={() => props.onSelecting(!props.selecting)}
            />
            <IconButton icon="magnifying-glass-minus" label={t("atlas.zoom.out")} density="compact" onClick={() => props.zoom.zoomBy(1 / 1.25)} />
            <IconButton icon="magnifying-glass-plus" label={t("atlas.zoom.in")} density="compact" onClick={() => props.zoom.zoomBy(1.25)} />
            <IconButton icon="expand" label={t("atlas.zoom.fit")} density="compact" onClick={props.zoom.fit} />
        </div>
    );
}

/** The map's selection: which nodes are in the Swarm's selection, and the box and the clicks that put them there. */
function useAtlasSelection(data: AtlasData, zoom: AtlasPanZoom, layout: () => AtlasLayout) {
    const model = data.model;
    const [selecting, setSelecting] = createSignal(false);
    /** The Swarm's selection key of each node drawn that it can select, by the node's own key. */
    const keys = createMemo(() => {
        const map = new Map<string, string>();
        for (const region of data.graph().regions) {
            for (const agent of region.shown) {
                const key = selectionKeyOf(agent);
                if (key) map.set(agent.key, key);
            }
        }
        return map;
    });
    const nodeKeys = createMemo(() => [...keys().values()]);
    const lasso = useAtlasLasso({
        selecting,
        view: zoom.view,
        nodes: () => layout().nodes,
        onPick: (touched) => model.setManySelected(touched.flatMap((k) => keys().get(k) ?? []), true),
    });
    const picked = (agent: AtlasAgent) => {
        const key = keys().get(agent.key);
        return !!key && (model.selectedBlockIdsAtom().has(key) || lasso.touched().has(agent.key));
    };
    /** A click that selects rather than opens the card: with Select on, or with Shift, Ctrl or Cmd held. */
    const toggleFor = (agentKey: string, e: MouseEvent | KeyboardEvent): boolean => {
        const key = keys().get(agentKey);
        if (!key || !(selecting() || e.shiftKey || e.ctrlKey || e.metaKey)) return false;
        model.toggleSelected(key);
        return true;
    };
    return { selecting, setSelecting, nodeKeys, lasso, picked, toggleFor };
}

/** The box a drag draws to select agents, in the pane's own pixels. */
function LassoBox(props: { rect: { x: number; y: number; w: number; h: number } | null }): JSX.Element {
    return (
        <Show when={props.rect}>
            {(r) => (
                <div
                    class="atlas-lasso"
                    data-testid="atlas-lasso"
                    style={{ left: `${r().x}px`, top: `${r().y}px`, width: `${r().w}px`, height: `${r().h}px` }}
                />
            )}
        </Show>
    );
}

/** Each machine's host and its ring (drawn only when there is more than one host to tell apart, atlas-hosts.ts), and the stopped agents with their hosts. */
function useAtlasHosts(data: AtlasData) {
    const hosts = createMemo(() => hostStyles(data.graph().regions));
    const ring = createMemo(() => ringsDrawn(hosts()));
    /** The agents the map does not draw, each with the host it is on. */
    const stopped = createMemo(() =>
        data.graph().regions.flatMap((r) => r.folded.map((agent) => ({ agent, host: hosts().get(r.key)?.name ?? "" })))
    );
    return { hosts, ring, stopped };
}

/** What the map leaves out or could not read, in its bottom left corner: the cloud, a machine's own trouble, the stopped agents. */
function AtlasNotes(props: {
    data: AtlasData;
    hosts: ReadonlyMap<string, HostStyle>;
    stopped: { agent: AtlasAgent; host: string }[];
    stoppedCount: number;
    /** The legend is drawn: it already says each host's trouble, so the corner does not repeat it over the nodes. */
    legend: boolean;
}): JSX.Element {
    return (
        <div class="atlas-notes">
            <Show when={props.data.cloudUnavailable()}>
                <div data-testid="atlas-cloud-note">{t("atlas.cloudNote")}</div>
            </Show>
            {/* What is wrong with a machine itself (its report is old, its container runtime is not running): said by its
                host's name here with one host, and in the legend beside the host with more. */}
            <For each={props.legend ? [] : props.data.graph().regions.filter((r) => r.note || r.stale)}>
                {(r) => (
                    <div data-testid="atlas-machine-note">
                        {props.hosts.get(r.key)?.name ?? r.title}: {machineDetail(r)}
                    </div>
                )}
            </For>
            <Show when={props.stoppedCount > 0}>
                <StoppedChip agents={props.stopped} />
            </Show>
        </div>
    );
}

/** The agent a node key names, if there is one (a key can also name a folded or extra node). */
function agentOf(data: AtlasData, key: string | null): AtlasAgent | undefined {
    return key ? data.agents().get(key) : undefined;
}

/** Each shown agent's tasks for its node, and the node heights they make when listed line by line. */
function useNodeTasks(data: AtlasData) {
    const tasks = useAtlasTasks();
    const nodeTaskLists = createMemo(() =>
        nodeTasksOf(
            data.graph().regions.flatMap((r) => r.shown),
            tasks()
        )
    );
    const lineHeights = createMemo(() => nodeHeights(nodeTaskLists()));
    return { tasks, nodeTaskLists, lineHeights };
}

/** How a machine was found and what it says of itself, for the card of an agent on it. */
function machineDetailOf(data: AtlasData, machine: string): string {
    const m = data.graph().regions.find((r) => r.key === machine);
    return m ? machineDetail(m) : "";
}

export function AtlasView(props: { data: AtlasData; ctx: PaneTabHostContext }): JSX.Element {
    const { data, ctx } = props;
    const mode = createMemo<AtlasLayoutMode>(() => readAtlasLayout(ctx.meta()));
    const [selected, setSelected] = createSignal<string | null>(null);
    // The pane is measured before the first draw and the layout is formed for its shape, so the first paint is
    // already fitted (the view reads the layout lazily: it is made right below).
    const zoom = useAtlasPanZoom(() => layout(), mode, () => setSelected(null));
    // Zoomed in past the fitted view, the gaps close up (`zoom.spread`), so more of the cards are in the pane.
    const { tasks, nodeTaskLists, lineHeights } = useNodeTasks(data);
    const layoutFor = (heights: NodeHeights, spread: number) =>
        mode() === "seat"
            ? layoutBySeat(data.graph(), zoom.shape(), spread, heights)
            : layoutAtlas(data.graph(), zoom.shape(), spread, heights);
    // Nodes list their tasks line by line when the fitted map with the lines is big enough to read them, or the person has
    // zoomed in to a scale that is. Otherwise a node says how many it has, and keeps the height every node has.
    const fitWithLines = createMemo(() => (lineHeights().size === 0 ? 0 : fitScale(layoutFor(lineHeights(), 1), zoom.shape())));
    const lines = createMemo(() => lineHeights().size > 0 && showTaskLines(fitWithLines(), zoom.touched(), zoom.view().k));
    const layout = createMemo<AtlasLayout>(() => layoutFor(lines() ? lineHeights() : NO_NODE_HEIGHTS, zoom.spread()));
    const sel = useAtlasSelection(data, zoom, layout);
    const { hosts, ring, stopped } = useAtlasHosts(data);
    const shownCount = createMemo(() => data.graph().regions.reduce((n, r) => n + r.shown.length + r.orphans.length, 0));
    const stoppedCount = createMemo(() => data.graph().regions.reduce((n, r) => n + r.folded.length, 0));
    const selectedAgent = () => agentOf(data, selected());
    const onPointerDown = (e: PointerEvent & { currentTarget: HTMLDivElement }) => {
        if (!sel.lasso.wants(e) || (e.target as HTMLElement).closest(NOT_BACKGROUND)) return zoom.onPointerDown(e);
        setSelected(null);
        sel.lasso.start(e, e.currentTarget);
    };
    const onPointerUp = (e: PointerEvent) => {
        sel.lasso.onPointerUp(e);
        zoom.onPointerUp(e);
    };
    // Branch and tasks are this instance's facts: an agent of another instance has no pane here to ask about.
    const work = useAtlasWork(() => {
        const agent = selectedAgent();
        return agent && !agent.remote ? agent : null;
    }, tasks);
    const onEscape = () => (sel.lasso.rect() ? sel.lasso.cancel() : setSelected(null));
    return (
        <div class="atlas" data-testid="atlas" onKeyDown={(e) => e.key === "Escape" && onEscape()}>
            <AtlasBar data={data} ctx={ctx} mode={mode()} zoom={zoom} selecting={sel.selecting()} onSelecting={sel.setSelecting} />
            {/* Each host's ring beside its name: a row only when there is a ring to explain. */}
            <Show when={ring()}>
                <AtlasLegend styles={hosts()} regions={data.graph().regions} />
            </Show>
            <div
                class="atlas-viewport"
                classList={{ "atlas-viewport--selecting": sel.selecting() }}
                ref={zoom.setViewport}
                data-testid="atlas-viewport"
                onWheel={zoom.onWheel}
                onPointerDown={onPointerDown}
                onPointerMove={(e) => {
                    sel.lasso.onPointerMove(e);
                    zoom.onPointerMove(e);
                }}
                onPointerUp={onPointerUp}
                onPointerCancel={onPointerUp}
            >
                <Show when={shownCount() > 0 || stoppedCount() > 0} fallback={<div class="atlas-empty">{t("atlas.empty")}</div>}>
                    <AtlasMap
                        data={data}
                        layout={layout()}
                        view={zoom.view()}
                        hosts={hosts()}
                        ring={ring()}
                        selected={selected()}
                        picked={sel.picked}
                        tasks={nodeTaskLists()}
                        lines={lines()}
                        onSelect={(key, e) => sel.toggleFor(key, e) || setSelected(key)}
                    />
                </Show>
                <LassoBox rect={sel.lasso.rect()} />
                <AtlasNotes
                    data={data}
                    hosts={hosts()}
                    stopped={stopped()}
                    stoppedCount={stoppedCount()}
                    legend={ring()}
                />
                <Show when={selectedAgent()} fallback={<ExtraCard graph={data.graph()} selected={selected()} onClose={() => setSelected(null)} />}>
                    {(agent) => (
                        <Card
                            agent={agent()}
                            machineTitle={hosts().get(agent().machine)?.name ?? data.machines().get(agent().machine)?.title ?? ""}
                            machineDetail={machineDetailOf(data, agent().machine)}
                            agents={data.agents()}
                            work={work()}
                            onClose={() => setSelected(null)}
                        />
                    )}
                </Show>
            </div>
            {/* Under the map, not over it: the map's top left stays put when the first agent is selected and this appears. */}
            <AtlasFleetBar model={data.model} swarm={data.swarm} selecting={sel.selecting()} nodeKeys={sel.nodeKeys} />
        </div>
    );
}
