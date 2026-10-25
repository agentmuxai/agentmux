// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Memory view: AgentMux's total, and the four parts it is made of (Core,
// Panes, Agents, Not placed), each opening to what is in it
// (SPEC_TOWER_MEMORY_FIRST_AND_AGENTMUX_BREAKDOWN_2026_10_25.md §4).

import { IconButton } from "@/app/element/ui";
import type { TowerMemory, TowerMemoryPart, TowerMemoryPartId, TowerMemoryRow } from "@/app/store/rpc-api";
import clsx from "clsx";
import { createEffect, For, type JSX, Show } from "solid-js";
import { KindIcon } from "./tower-kind-ui";
import type { TowerViewModel } from "./tower-model";
import { count, formatMem } from "./tower-util";

/** What each part is called, and what is in it. */
const PARTS: Record<TowerMemoryPartId, { label: string; hint: string }> = {
    core: {
        label: "Core",
        hint: "AgentMux's own programs: the launcher, the server, the browser host and its GPU and service processes",
    },
    panes: {
        label: "Panes",
        hint: "What draws and hosts the panes: the window renderers, and each terminal with what runs in it",
    },
    agents: {
        label: "Agents",
        hint: "Each agent: its CLI, what runs for it (MCP servers, helpers), and what it started",
    },
    not_placed: {
        label: "Not placed",
        hint: "AgentMux started these and no pane claims them",
    },
};

/** A part's share of the whole, as a word: "44%", "<1%". */
export function formatShare(part: number, whole: number): string {
    if (whole <= 0 || part <= 0) return "0%";
    const pct = (part / whole) * 100;
    return pct < 1 ? "<1%" : `${Math.round(pct)}%`;
}

const partKey = (id: TowerMemoryPartId) => `memory:part:${id}`;
const rowKey = (key: string) => `memory:row:${key}`;

/** Open the agent a row stands for in the Agents view. */
function showInAgents(m: TowerViewModel, row: TowerMemoryRow, part: TowerMemoryPartId) {
    if (!row.task) return;
    m.select(part === "panes" ? "terminals" : row.task);
    m.setView("agents");
}

export function MemoryView(props: { model: TowerViewModel }): JSX.Element {
    const m = props.model;
    const memory = () => m.snapshot()?.memory;
    // The busiest part is open the first time, as the Type grouping's are.
    createEffect(() => {
        const v = memory();
        if (!v || m.memoryOpened) return;
        const busiest = [...v.parts].sort((a, b) => b.mem - a.mem)[0];
        if (!busiest || busiest.mem <= 0) return;
        m.memoryOpened = true;
        m.open([partKey(busiest.part)]);
    });
    return (
        <Show when={memory()} fallback={<div class="tower-empty">Measuring…</div>}>
            {(v) => (
                <div class="tower-memory" data-testid="tower-memory">
                    <PartsBar model={m} memory={v()} />
                    <PartsTable model={m} memory={v()} />
                    <Footnote memory={v()} />
                </div>
            )}
        </Show>
    );
}

/** One bar split into the four parts: each a button that opens its part. */
function PartsBar(props: { model: TowerViewModel; memory: TowerMemory }): JSX.Element {
    const describe = (p: TowerMemoryPart) =>
        `${PARTS[p.part].label}: ${formatMem(p.mem)}, ${formatShare(p.mem, props.memory.total)}`;
    return (
        <div class="tower-memory-bar-wrap">
            <div
                class="tower-memory-bar"
                role="group"
                aria-label={`AgentMux's memory by part: ${props.memory.parts.map(describe).join("; ")}`}
            >
                <For each={props.memory.parts}>
                    {(p) => (
                        <Show when={p.mem > 0 && props.memory.total > 0}>
                            <div
                                role="button"
                                tabindex="0"
                                class={clsx("tower-memory-segment", `tower-memory-part--${p.part}`)}
                                style={{ width: `${(p.mem / props.memory.total) * 100}%` }}
                                title={`${describe(p)}. ${PARTS[p.part].hint}`}
                                aria-label={describe(p)}
                                data-testid={`tower-memory-segment-${p.part}`}
                                onClick={() => props.model.open([partKey(p.part)])}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter" || e.key === " ") {
                                        e.preventDefault();
                                        props.model.open([partKey(p.part)]);
                                    }
                                }}
                            />
                        </Show>
                    )}
                </For>
            </div>
            <div class="tower-memory-legend">
                <For each={props.memory.parts}>
                    {(p) => (
                        <span class="tower-memory-key" title={PARTS[p.part].hint}>
                            <span
                                class={clsx("tower-memory-swatch", `tower-memory-part--${p.part}`)}
                                aria-hidden="true"
                            />
                            <span>{PARTS[p.part].label}</span>
                            <span class="tower-num tower-muted">{formatMem(p.mem)}</span>
                        </span>
                    )}
                </For>
            </div>
        </div>
    );
}

function PartsTable(props: { model: TowerViewModel; memory: TowerMemory }): JSX.Element {
    return (
        <table class="tower-table tower-memory-table" aria-label="Memory by part">
            <thead>
                <tr>
                    <th>Part</th>
                    <th class="tower-num">Memory</th>
                    <th class="tower-num">Share</th>
                </tr>
            </thead>
            <tbody>
                <For each={props.memory.parts}>
                    {(p) => <PartRows model={props.model} memory={props.memory} part={p} />}
                </For>
            </tbody>
        </table>
    );
}

function PartRows(props: { model: TowerViewModel; memory: TowerMemory; part: TowerMemoryPart }): JSX.Element {
    const m = props.model;
    const key = () => partKey(props.part.part);
    const open = () => m.expanded().has(key());
    const total = () => props.memory.total;
    return (
        <>
            <tr
                class={clsx("tower-group-row tower-memory-part", `tower-memory-part--${props.part.part}`)}
                data-testid={`tower-memory-part-${props.part.part}`}
                title={PARTS[props.part.part].hint}
            >
                <td class="tower-name">
                    <div class="tower-name-line">
                        <Show
                            when={props.part.rows.length > 0}
                            fallback={<span class="tower-memory-spacer" aria-hidden="true" />}
                        >
                            <IconButton
                                icon={open() ? "chevron-down" : "chevron-right"}
                                label={
                                    open()
                                        ? `Hide what's in ${PARTS[props.part.part].label}`
                                        : `Show what's in ${PARTS[props.part.part].label}`
                                }
                                density="compact"
                                tooltip={false}
                                aria-expanded={open()}
                                onClick={() => m.toggleExpanded(key())}
                            />
                        </Show>
                        <span class="tower-memory-swatch" aria-hidden="true" />
                        <span class="tower-label">{PARTS[props.part.part].label}</span>
                        <span class="tower-muted">({props.part.processes})</span>
                    </div>
                </td>
                <td class="tower-num">{formatMem(props.part.mem)}</td>
                <td class="tower-num tower-muted">{formatShare(props.part.mem, total())}</td>
            </tr>
            <Show when={open()}>
                <Show when={props.part.part === "not_placed" && props.part.rows.length > 0}>
                    <tr class="tower-memory-hint">
                        <td colspan={3} class="tower-muted">
                            {PARTS.not_placed.hint}.
                        </td>
                    </tr>
                </Show>
                <For each={props.part.rows}>
                    {(r) => <ItemRows model={m} memory={props.memory} part={props.part.part} row={r} />}
                </For>
            </Show>
        </>
    );
}

function ItemRows(props: {
    model: TowerViewModel;
    memory: TowerMemory;
    part: TowerMemoryPartId;
    row: TowerMemoryRow;
}): JSX.Element {
    const m = props.model;
    const key = () => rowKey(props.row.key);
    const open = () => m.expanded().has(key());
    const total = () => props.memory.total;
    const goes = () => !!props.row.task && props.part !== "core";
    return (
        <>
            <tr
                class={clsx("tower-process-row tower-memory-row", props.row.kind && `tower-kind--${props.row.kind}`)}
                data-testid={`tower-memory-row-${props.row.key}`}
            >
                <td class="tower-name">
                    <div class="tower-name-line">
                        <Show
                            when={props.row.children.length > 0}
                            fallback={<span class="tower-memory-spacer" aria-hidden="true" />}
                        >
                            <IconButton
                                icon={open() ? "chevron-down" : "chevron-right"}
                                label={open() ? `Hide ${props.row.label}'s parts` : `Show ${props.row.label}'s parts`}
                                density="compact"
                                tooltip={false}
                                aria-expanded={open()}
                                onClick={() => m.toggleExpanded(key())}
                            />
                        </Show>
                        <KindIcon kind={props.row.kind} />
                        <span class="tower-label tower-memory-label">{props.row.label}</span>
                        <Show when={props.row.processes > 1}>
                            <span class="tower-muted">×{props.row.processes}</span>
                        </Show>
                        <Show when={goes()}>
                            <IconButton
                                icon="arrow-up-right-from-square"
                                label={`Show ${props.row.label} in the Agents view`}
                                density="compact"
                                onClick={() => showInAgents(m, props.row, props.part)}
                            />
                        </Show>
                    </div>
                </td>
                <td class="tower-num">{formatMem(props.row.mem)}</td>
                <td class="tower-num tower-muted">{formatShare(props.row.mem, total())}</td>
            </tr>
            <Show when={open()}>
                <For each={props.row.children}>
                    {(c) => (
                        <tr class="tower-memory-child" data-testid={`tower-memory-child-${c.key}`}>
                            <td class="tower-name">
                                <div class="tower-name-line">
                                    <span class="tower-label">{c.label}</span>
                                    <Show when={c.processes > 1}>
                                        <span class="tower-muted">×{c.processes}</span>
                                    </Show>
                                </div>
                            </td>
                            <td class="tower-num">{formatMem(c.mem)}</td>
                            <td class="tower-num tower-muted">{formatShare(c.mem, total())}</td>
                        </tr>
                    )}
                </For>
            </Show>
        </>
    );
}

/** What the number is, and what is left out of it. */
function Footnote(props: { memory: TowerMemory }): JSX.Element {
    return (
        <div class="tower-memory-note tower-muted" data-testid="tower-memory-note">
            <div>
                {count(props.memory.processes, "process")}, measured as {props.memory.metric}: the memory only
                AgentMux's processes use, added up over everything AgentMux started. Shared libraries and shared memory
                aren't counted twice, and video memory isn't counted.
            </div>
            <Show when={props.memory.unreadable > 0}>
                <div>{count(props.memory.unreadable, "process")} couldn't be read and add nothing.</div>
            </Show>
            <Show when={(props.memory.other_instances ?? 0) > 0}>
                <div data-testid="tower-memory-others">
                    Another AgentMux on this computer uses {formatMem(props.memory.other_instances)}, not counted here.
                </div>
            </Show>
        </div>
    );
}
