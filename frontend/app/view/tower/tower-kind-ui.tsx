// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a process's kind and load look like (SPEC_TOWER_THEME_SURFACE_AND_COLOR_2026_10_17.md
// §3.3, §5.4): the kind's icon before a name, a CPU or memory figure with a
// bar for its share of the machine, and the breakdown of the machine by kind
// with a chip per kind that filters the list.

import { Button } from "@/app/element/ui";
import type { TowerProcess, TowerProcessKind } from "@/app/store/rpc-api";
import { ProviderLogo } from "@/element/ProviderLogo";
import clsx from "clsx";
import { createMemo, For, type JSX, Match, Show, Switch } from "solid-js";
import { type KindGroup, kindGroups, KINDS } from "./tower-kinds";
import type { TowerViewModel } from "./tower-model";
import { count, formatMem, type LoadLevel, loadLevel, type LoadScope } from "./tower-util";

/** A kind's icon in its colour; an agent's own process shows its provider's
 *  logo, AgentMux's processes AgentMux's. Nothing for a row without a kind
 *  (an AgentMux that predates kinds). */
export function KindIcon(props: { kind?: TowerProcessKind; provider?: string }): JSX.Element {
    return (
        <Show when={props.kind}>
            {(kind) => (
                <span
                    class={clsx("tower-kind-icon", `tower-kind--${kind()}`)}
                    role="img"
                    aria-label={KINDS[kind()].label}
                    title={KINDS[kind()].label}
                >
                    <Switch
                        fallback={
                            <i
                                class={`fa ${KINDS[kind()].regular ? "fa-regular" : "fa-solid"} fa-${KINDS[kind()].icon}`}
                            />
                        }
                    >
                        <Match when={kind() === "agents" && props.provider}>
                            {(provider) => <ProviderLogo provider={provider()} size={12} />}
                        </Match>
                        <Match when={kind() === "agentmux"}>
                            <ProviderLogo provider="agentmux" size={12} />
                        </Match>
                    </Switch>
                </span>
            )}
        </Show>
    );
}

/** A number cell with a bar under its figure: the bar's length is the share
 *  of the machine, drawn in the row's kind colour (`--tower-kind-fill`, from
 *  a `tower-kind--*` class on the row) or `fill`. A heavy figure takes its
 *  level's colour. */
export function LoadCell(props: {
    text: string;
    share?: number;
    level?: LoadLevel;
    fill?: string;
    title?: string;
}): JSX.Element {
    const width = () => Math.min(100, Math.max(0, (props.share ?? 0) * 100));
    return (
        <td class={clsx("tower-num tower-load", props.level && `tower-level--${props.level}`)} title={props.title}>
            {props.text}
            <Show when={width() > 0}>
                <span
                    class="tower-load-bar"
                    aria-hidden="true"
                    style={{ width: `${width()}%`, ...(props.fill ? { "--tower-load-fill": props.fill } : {}) }}
                />
            </Show>
        </td>
    );
}

/** A row's CPU and Memory cells, each with its bar and level. `fill` colours
 *  the bars where the row has no kind of its own (an agent's group: its
 *  colour). */
export function LoadCells(props: {
    model: TowerViewModel;
    cpu: (f: number | undefined) => string;
    cpuValue: number | undefined;
    mem: number | undefined;
    scope?: LoadScope;
    fill?: string;
    title?: string;
}): JSX.Element {
    const scope = () => props.scope ?? "item";
    const cpuShare = () => props.model.cpuShare(props.cpuValue);
    const memShare = () => props.model.memShare(props.mem);
    return (
        <>
            <LoadCell
                text={props.cpu(props.cpuValue)}
                share={cpuShare()}
                level={loadLevel("cpu", scope(), cpuShare())}
                fill={props.fill}
                title={props.title}
            />
            <LoadCell
                text={formatMem(props.mem)}
                share={memShare()}
                level={loadLevel("mem", scope(), memShare())}
                fill={props.fill}
                title={props.title}
            />
        </>
    );
}

/** One stacked bar: a segment per kind, each its share of `whole`. */
function KindBar(props: {
    label: string;
    groups: KindGroup[];
    value: (g: KindGroup) => number | undefined;
    whole: number;
    describe: (g: KindGroup) => string;
    unlisted?: number;
    total: string;
}) {
    const share = (v: number | undefined) => (v == null || props.whole <= 0 ? 0 : Math.min(1, v / props.whole));
    return (
        <div class="tower-kind-bar-row">
            <span class="tower-kind-bar-label">{props.label}</span>
            <div class="tower-kind-bar" role="img" aria-label={`${props.label} by kind of process: ${props.total}`}>
                <For each={props.groups}>
                    {(g) => (
                        <Show when={share(props.value(g)) > 0}>
                            <span
                                class={clsx("tower-kind-segment", `tower-kind--${g.kind}`)}
                                style={{ width: `${share(props.value(g)) * 100}%` }}
                                title={props.describe(g)}
                            />
                        </Show>
                    )}
                </For>
                <Show when={share(props.unlisted) > 0}>
                    <span
                        class="tower-kind-segment tower-kind-segment--unlisted"
                        style={{ width: `${share(props.unlisted) * 100}%` }}
                        title="Processes the other machine didn't list"
                    />
                </Show>
            </div>
            <span class="tower-kind-bar-total tower-num">{props.total}</span>
        </div>
    );
}

/** Where the machine's CPU and memory go, by kind of process, and a chip per
 *  kind that shows only that kind (again to show all). */
export function KindBreakdown(props: { model: TowerViewModel; cpu: (f: number | undefined) => string }): JSX.Element {
    const m = props.model;
    const host = () => m.snapshot()?.host;
    const processes = (): TowerProcess[] => host()?.processes ?? [];
    const groups = createMemo(() => kindGroups(processes()));
    const cpuCount = () => m.snapshot()?.cpu_count ?? 1;
    const memTotal = () => m.snapshot()?.machine?.mem_total;
    // Another machine lists only its busiest and largest: what the rest use
    // is the machine's total less the listed rows'.
    const listedCpu = () => groups().reduce((n, g) => n + (g.cpu ?? 0), 0);
    const listedMem = () => groups().reduce((n, g) => n + (g.mem ?? 0), 0);
    const partial = () => processes().length < (host()?.matched ?? 0);
    const describe = (g: KindGroup) =>
        `${KINDS[g.kind].label} · ${count(g.processes.length, "process")} · CPU ${props.cpu(g.cpu)} · ${formatMem(g.mem)}`;
    return (
        <Show when={processes().some((p) => p.kind)}>
            <div class="tower-kinds" data-testid="tower-kinds">
                <KindBar
                    label="CPU"
                    groups={groups()}
                    value={(g) => g.cpu}
                    whole={cpuCount()}
                    describe={describe}
                    unlisted={partial() ? Math.max(0, (host()?.cpu ?? 0) - listedCpu()) : undefined}
                    total={props.cpu(host()?.cpu)}
                />
                <KindBar
                    label="Memory"
                    groups={groups()}
                    value={(g) => g.mem}
                    // Of installed memory where the machine says, else of what
                    // all processes use.
                    whole={memTotal() ?? host()?.mem ?? 0}
                    describe={describe}
                    unlisted={partial() ? Math.max(0, (host()?.mem ?? 0) - listedMem()) : undefined}
                    total={
                        memTotal() ? `${formatMem(host()?.mem)} of ${formatMem(memTotal())}` : formatMem(host()?.mem)
                    }
                />
                <div class="tower-kind-chips" role="group" aria-label="Show one kind of process">
                    <For each={groups()}>
                        {(g) => (
                            <Button
                                density="compact"
                                tone="quiet"
                                class={clsx("tower-kind-chip", `tower-kind--${g.kind}`)}
                                pressed={m.kindFilter() === g.kind}
                                title={`${describe(g)}. ${m.kindFilter() === g.kind ? "Show every kind" : "Show only these"}`}
                                onClick={() => m.setKindFilter(m.kindFilter() === g.kind ? "" : g.kind)}
                                data-testid={`tower-kind-chip-${g.kind}`}
                            >
                                <KindIcon kind={g.kind} />
                                <span>{KINDS[g.kind].short}</span>
                                <span class="tower-muted tower-num">
                                    {g.processes.length} · {props.cpu(g.cpu)}
                                </span>
                            </Button>
                        )}
                    </For>
                </div>
            </div>
        </Show>
    );
}
