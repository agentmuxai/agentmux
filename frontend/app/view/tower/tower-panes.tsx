// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tower's Panes view: this machine's AgentMux by window, tab and pane, each
// pane with what it costs (SPEC_TOWER_PANES_VIEW_2026_10_21.md). Measured
// numbers only (phase 1): an agent's or terminal's processes, a browser pane's
// page renderers; a pane drawn in its window's renderer shows a dash, and the
// window's row carries that renderer.

import { EmptyState, IconButton } from "@/app/element/ui";
import type { TowerPane, TowerPaneTab, TowerPaneWindow, TowerProcess } from "@/app/store/rpc-api";
import { revealBlock } from "@/app/util/reveal-block";
import clsx from "clsx";
import { createMemo, For, type JSX, Show } from "solid-js";
import { LoadCells } from "./tower-kind-ui";
import type { TowerViewModel } from "./tower-model";
import { ProcessName } from "./tower-process-name";
import { formatMem, processDetail } from "./tower-util";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";

type Cpu = (f: number | undefined) => string;

/** What a pane's view is, in words, for its row: the views that have a name of their own (built when drawn, so a change of language shows). */
const VIEW_KEYS = new Set([
    "agent",
    "term",
    "browser",
    "editor",
    "files",
    "sysinfo",
    "swarm",
    "tower",
    "settings",
    "warden",
    "history",
    "audit",
    "memory",
    "toolchain",
    "connectors",
    "launcher",
]);

export function viewName(view: string): string {
    if (VIEW_KEYS.has(view)) return t(`tower.panes.view.${view}`);
    return view ? view.charAt(0).toUpperCase() + view.slice(1) : t("tower.panes.view.pane");
}

/** A pane's name in its row: its title, else what kind of pane it is. */
export function paneName(p: TowerPane): string {
    return p.title.trim() || viewName(p.view);
}

/** A pane's state in words: the three the app sends, else the word as it came. */
function stateWord(state: string): string {
    return state === "shown" || state === "hidden" || state === "asleep" ? t(`tower.panes.state.${state}`) : state;
}

/** Why a pane has no number: it is drawn in its window's renderer. */
const inWindowTitle = () => t("tower.panes.inWindowTip");

export function PanesView(props: { model: TowerViewModel; cpu: Cpu }): JSX.Element {
    const m = props.model;
    const panes = () => m.snapshot()?.panes;
    return (
        <Show when={panes()} fallback={<EmptyState loading>{t("tower.panes.measuring")}</EmptyState>}>
            {(v) => (
                <>
                    <div class="tower-summary">
                        {t("tower.panes.summary", {
                            windows: t("tower.panes.windows", { count: v().windows.length }),
                            metric: m.snapshot()?.memory_metric ?? "",
                        })}
                        <Show when={v().shared_mem > 0}>
                            <span class="tower-muted">
                                {" "}
                                · {t("tower.panes.sharedTotal", { mem: formatMem(v().shared_mem) })}
                            </span>
                        </Show>
                    </div>
                    <table class="tower-table tower-panes" aria-label={t("tower.panes.aria")}>
                        <thead>
                            <tr>
                                <th>{t("tower.panes.col.pane")}</th>
                                <th>{t("tower.panes.col.state")}</th>
                                <th class="tower-num">{t("tower.panes.col.memory")}</th>
                                <th class="tower-num">{t("tower.panes.col.cpu")}</th>
                            </tr>
                        </thead>
                        <tbody>
                            <For each={v().windows}>
                                {(w) => <WindowRows model={m} window={w} cpu={props.cpu} />}
                            </For>
                            <Show when={v().pool.length > 0}>
                                <PoolRows model={m} pool={v().pool} cpu={props.cpu} />
                            </Show>
                        </tbody>
                    </table>
                    <div class="tower-panes-note tower-muted">
                        {t("tower.panes.note")}
                    </div>
                </>
            )}
        </Show>
    );
}

/** One process under a window, a pane or the pre-warmed windows: its name
 *  and PID, then its CPU and memory in this table's columns. */
function PaneProcessRow(props: { model: TowerViewModel; process: TowerProcess; cpu: Cpu; depth: 1 | 2 }) {
    return (
        <tr
            class={clsx(
                "tower-process-row",
                props.depth === 2 ? "tower-panes-proc--deep" : "tower-panes-proc",
                props.process.kind && `tower-kind--${props.process.kind}`
            )}
            {...tip(processDetail(props.process, props.model.snapshot()?.memory_metric ?? ""))}
        >
            <td class="tower-name">
                <div class="tower-name-line">
                    <ProcessName process={props.process}>
                        <span class="tower-muted">{props.process.pid}</span>
                    </ProcessName>
                </div>
            </td>
            <td />
            <LoadCells model={props.model} cpu={props.cpu} cpuValue={props.process.cpu} mem={props.process.mem} />
        </tr>
    );
}

function WindowRows(props: { model: TowerViewModel; window: TowerPaneWindow; cpu: Cpu }) {
    const m = props.model;
    const key = () => `pwin:${props.window.window_id}`;
    // Windows start open; closing one is remembered.
    const open = () => !m.expanded().has(key());
    const name = () => props.window.name || t("tower.panes.window");
    return (
        <>
            <tr class="tower-owner-row tower-panes-window" data-testid={`tower-pane-window-${props.window.window_id}`}>
                <td class="tower-name">
                    <div class="tower-name-line">
                        <IconButton
                            icon={open() ? "chevron-down" : "chevron-right"}
                            label={open() ? t("tower.panes.hideTabs", { name: name() }) : t("tower.panes.showTabs", { name: name() })}
                            density="compact"
                            tooltip={false}
                            aria-expanded={open()}
                            onClick={() => m.toggleExpanded(key())}
                        />
                        <i class="fa-solid fa-window-maximize" aria-hidden="true" />
                        <span class="tower-label">{t("tower.panes.windowName", { name: name() })}</span>
                    </div>
                </td>
                <td class="tower-muted" {...tip(t("tower.panes.rendererTip"))}>
                    {t("tower.panes.renderer")}
                </td>
                <LoadCells
                    model={m}
                    cpu={props.cpu}
                    cpuValue={props.window.cpu}
                    mem={props.window.mem}
                    scope="item"
                    title={t("tower.panes.rendererTip")}
                />
            </tr>
            <Show when={open()}>
                <For each={props.window.tabs}>{(tab) => <TabRows model={m} tab={tab} cpu={props.cpu} />}</For>
                <For each={props.window.renderers}>
                    {(p) => <PaneProcessRow model={m} process={p} cpu={props.cpu} depth={1} />}
                </For>
            </Show>
        </>
    );
}

function TabRows(props: { model: TowerViewModel; tab: TowerPaneTab; cpu: Cpu }) {
    return (
        <>
            <tr class="tower-panes-tab" data-testid={`tower-pane-tab-${props.tab.tab_id}`}>
                <td class="tower-name">
                    <div class="tower-name-line">
                        <span class="tower-label">{props.tab.name || t("tower.panes.tab")}</span>
                    </div>
                </td>
                <td class={clsx(!props.tab.shown && "tower-muted")}>{props.tab.shown ? t("tower.panes.state.shown") : t("tower.panes.state.hidden")}</td>
                <td />
                <td />
            </tr>
            <For each={props.tab.panes}>{(p) => <PaneRows model={props.model} pane={p} cpu={props.cpu} />}</For>
        </>
    );
}

function PaneRows(props: { model: TowerViewModel; pane: TowerPane; cpu: Cpu }) {
    const m = props.model;
    const key = () => `pane:${props.pane.block_id}`;
    const task = createMemo(() =>
        props.pane.task ? m.snapshot()?.tasks.find((task) => task.id === props.pane.task) : undefined
    );
    const processes = createMemo<TowerProcess[]>(() => task()?.processes ?? props.pane.processes);
    const expandable = () => processes().length > 0 || props.pane.shared.length > 0;
    const open = () => expandable() && m.expanded().has(key());
    const measured = () => props.pane.task != null || props.pane.processes.length > 0;
    const asleep = () => props.pane.state === "asleep";
    return (
        <>
            <tr
                class={clsx("tower-process-row tower-panes-pane", asleep() && "tower-panes-pane--asleep")}
                data-testid={`tower-pane-${props.pane.block_id}`}
            >
                <td class="tower-name">
                    <div class="tower-name-line tower-panes-pane-name">
                        <Show when={expandable()} fallback={<span class="tower-panes-spacer" />}>
                            <IconButton
                                icon={open() ? "chevron-down" : "chevron-right"}
                                label={open() ? t("tower.panes.hideProcesses") : t("tower.panes.showProcesses")}
                                density="compact"
                                tooltip={false}
                                aria-expanded={open()}
                                onClick={() => m.toggleExpanded(key())}
                            />
                        </Show>
                        <span class="tower-label tower-panes-name">{paneName(props.pane)}</span>
                        <Show when={props.pane.title.trim()}>
                            <span class="tower-muted">{viewName(props.pane.view)}</span>
                        </Show>
                        <IconButton
                            icon="arrow-up-right-from-square"
                            label={t("tower.panes.show")}
                            density="compact"
                            onClick={() => void revealBlock(props.pane.block_id, { source: "tower" })}
                        />
                    </div>
                </td>
                <td class={clsx(props.pane.state !== "shown" && "tower-muted")}>
                    {stateWord(props.pane.state)}
                </td>
                <Show
                    when={measured()}
                    fallback={
                        <>
                            <td class="tower-num tower-muted" {...tip(asleep() ? undefined : inWindowTitle())}>
                                {asleep() ? t("tower.panes.noPage") : "—"}
                            </td>
                            <td class="tower-num tower-muted" {...tip(asleep() ? undefined : inWindowTitle())}>
                                —
                            </td>
                        </>
                    }
                >
                    <LoadCells model={m} cpu={props.cpu} cpuValue={props.pane.cpu} mem={props.pane.mem} />
                </Show>
            </tr>
            <Show when={props.pane.shared.length > 0}>
                <tr class="tower-panes-shared">
                    <td class="tower-name tower-muted" colspan={2}>
                        {t("tower.panes.sharesRenderer")}
                    </td>
                    <td class="tower-num tower-muted">
                        {formatMem(props.pane.shared.reduce((a, p) => a + (p.mem ?? 0), 0))}
                    </td>
                    <td />
                </tr>
            </Show>
            <Show when={open()}>
                <For each={processes()}>{(p) => <PaneProcessRow model={m} process={p} cpu={props.cpu} depth={2} />}</For>
                <For each={props.pane.shared}>
                    {(p) => <PaneProcessRow model={m} process={p} cpu={props.cpu} depth={2} />}
                </For>
            </Show>
        </>
    );
}

function PoolRows(props: { model: TowerViewModel; pool: TowerProcess[]; cpu: Cpu }) {
    const m = props.model;
    const key = "pool";
    const open = () => m.expanded().has(key);
    const cpu = () => {
        const v = props.pool.map((p) => p.cpu).filter((c): c is number => c != null);
        return v.length ? v.reduce((a, b) => a + b, 0) : undefined;
    };
    const mem = () => props.pool.reduce((a, p) => a + (p.mem ?? 0), 0);
    return (
        <>
            <tr class="tower-owner-row tower-panes-pool" data-testid="tower-pane-pool">
                <td class="tower-name">
                    <div class="tower-name-line">
                        <IconButton
                            icon={open() ? "chevron-down" : "chevron-right"}
                            label={open() ? t("tower.panes.hidePool") : t("tower.panes.showPool")}
                            density="compact"
                            tooltip={false}
                            aria-expanded={open()}
                            onClick={() => m.toggleExpanded(key)}
                        />
                        <span class="tower-label" {...tip(t("tower.panes.poolTip"))}>
                            {t("tower.panes.pool")}
                        </span>
                    </div>
                </td>
                <td class="tower-muted">{t("tower.panes.state.hidden")}</td>
                <LoadCells model={m} cpu={props.cpu} cpuValue={cpu()} mem={mem()} scope="item" />
            </tr>
            <Show when={open()}>
                <For each={props.pool}>{(p) => <PaneProcessRow model={m} process={p} cpu={props.cpu} depth={1} />}</For>
            </Show>
        </>
    );
}
