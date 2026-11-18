// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The GPU panel's views: the notice for what the counters said, the adapters, the processes, and the footer. */

import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { GpuAdapter } from "@/types/rpc/GpuAdapter";
import type { GpuProcess } from "@/types/rpc/GpuProcess";
import type { GpuSampleResult } from "@/types/rpc/GpuSampleResult";
import type { GpuInfo } from "@/util/gpuutil";
import { For, Index, Show, type JSX } from "solid-js";
import { barWidth, formatGpuBytes, formatPct, gpuColor, memoryText } from "./gpu-format";
import { MIN_POLL_MS } from "./gpu-panel-state";
import { UI_BUSY_WARN_PCT, uiBusyColor } from "./ui-busy";

/** What the sample says when it has no adapter to show: reading, nothing to read, or not read here yet. */
function stateNotice(s: GpuSampleResult | null, failed: boolean): string | null {
    if (s == null) return failed ? t("gpuPanel.failed") : t("gpuPanel.reading");
    switch (s.state) {
        case "warming":
            return t("gpuPanel.reading");
        case "unavailable":
            return t("gpuPanel.unavailable");
        case "unsupported":
            return t("gpuPanel.unsupported", { os: s.os });
        default:
            return s.adapters.length === 0 ? t("gpuPanel.noAdapter") : null;
    }
}

/** How the page's own graphics are drawn (hardware, software, off), with the renderer string the status item has. */
export function GfxLine(props: { info: GpuInfo }): JSX.Element {
    const text = () => {
        switch (props.info.classification) {
            case "hardware":
                return t("gpuPanel.gfx.hardware");
            case "software":
                return t("gpuPanel.gfx.software");
            default:
                return t("gpuPanel.gfx.unavailable");
        }
    };
    return (
        <div class="gpu-gfx" classList={{ "gpu-gfx-warn": props.info.classification !== "hardware" }}>
            <span>{text()}</span>
            <Show when={props.info.renderer}>
                <span class="gpu-renderer">{props.info.renderer}</span>
            </Show>
        </div>
    );
}

export function GpuNotice(props: { sample: GpuSampleResult | null; failed: boolean }): JSX.Element {
    return (
        <Show when={stateNotice(props.sample, props.failed)}>
            {(text) => <div class="gpu-panel-empty">{text()}</div>}
        </Show>
    );
}

function AdapterRow(props: { a: GpuAdapter }): JSX.Element {
    const a = () => props.a;
    const dedicated = () => memoryText(a().dedicated_used, a().dedicated_total);
    const shared = () => memoryText(a().shared_used, a().shared_total);
    const memShare = () => {
        const x = a();
        const [used, total] =
            x.kind === "discrete" ? [x.dedicated_used, x.dedicated_total] : [x.shared_used, x.shared_total];
        return used != null && total ? (used / total) * 100 : null;
    };
    return (
        <div class="gpu-adapter">
            <div class="gpu-adapter-head">
                <span class="gpu-adapter-name">{a().name}</span>
                <span class="gpu-adapter-kind">{t(`gpuPanel.kind.${a().kind}`)}</span>
                <Show when={a().use_pct != null}>
                    <span class="gpu-adapter-use" style={{ color: gpuColor(a().use_pct, memShare()) }}>
                        {formatPct(a().use_pct!)}
                    </span>
                </Show>
            </div>
            <Show when={a().use_pct != null}>
                <span class="gpu-bar" aria-hidden="true">
                    <span class="gpu-bar-fill" style={{ width: `${barWidth(a().use_pct)}%` }} />
                </span>
            </Show>
            <div class="gpu-adapter-memory">
                <Show when={dedicated()}>
                    <span>{t("gpuPanel.memory.dedicated", { amount: dedicated()! })}</span>
                </Show>
                <Show when={shared()}>
                    <span>{t("gpuPanel.memory.shared", { amount: shared()! })}</span>
                </Show>
            </div>
        </div>
    );
}

export function AdapterList(props: { adapters: GpuAdapter[] }): JSX.Element {
    return (
        <Show when={props.adapters.length > 0}>
            <div class="gpu-adapters">
                <Index each={props.adapters}>{(a) => <AdapterRow a={a()} />}</Index>
            </div>
        </Show>
    );
}

function memoryOf(p: { dedicated?: number; shared?: number }): number | null {
    return p.dedicated == null && p.shared == null ? null : (p.dedicated ?? 0) + (p.shared ?? 0);
}

function ProcessRow(props: { p: GpuProcess; onOpenTower: () => void }): JSX.Element {
    const p = () => props.p;
    const mem = () => memoryOf(p());
    const body = (
        <>
            <span class="gpu-process-name">
                {p().name}
                <Show when={p().label}>
                    <span class="gpu-process-label">{p().label}</span>
                </Show>
            </span>
            <span class="gpu-process-use">{p().use_pct != null ? formatPct(p().use_pct!) : ""}</span>
            <span class="gpu-process-mem">{mem() != null ? formatGpuBytes(mem()!) : ""}</span>
        </>
    );
    // AgentMux's own processes open Tower, where they are named the same way.
    return (
        <Show when={p().label} fallback={<div class="gpu-process">{body}</div>}>
            <Button
                variant="row"
                class="gpu-process gpu-process-own"
                onClick={props.onOpenTower}
                {...tip(t("gpuPanel.openTower"))}
            >
                {body}
            </Button>
        </Show>
    );
}

export function ProcessList(props: { sample: GpuSampleResult; onOpenTower: () => void }): JSX.Element {
    const s = () => props.sample;
    return (
        <Show when={s().state === "ok" && s().adapters.length > 0}>
            <div class="gpu-processes">
                <div class="gpu-section-title">{t("gpuPanel.processes")}</div>
                <Show
                    when={s().per_process}
                    fallback={<div class="gpu-panel-empty">{t("gpuPanel.noPerProcess")}</div>}
                >
                    <Show
                        when={s().processes.length > 0 || s().others}
                        fallback={<div class="gpu-panel-empty">{t("gpuPanel.noProcesses")}</div>}
                    >
                        <div class="gpu-process gpu-process-head" aria-hidden="true">
                            <span class="gpu-process-name">{t("gpuPanel.col.process")}</span>
                            <span class="gpu-process-use">{t("gpuPanel.col.use")}</span>
                            <span class="gpu-process-mem">{t("gpuPanel.col.memory")}</span>
                        </div>
                        <div class="gpu-process-list">
                            <For each={s().processes}>
                                {(p) => <ProcessRow p={p} onOpenTower={props.onOpenTower} />}
                            </For>
                            <Show when={s().others}>
                                {(o) => (
                                    <div class="gpu-process gpu-process-others">
                                        <span class="gpu-process-name">
                                            {t("gpuPanel.others", { count: o().processes })}
                                        </span>
                                        <span class="gpu-process-use">
                                            {o().use_pct != null ? formatPct(o().use_pct!) : ""}
                                        </span>
                                        <span class="gpu-process-mem">
                                            {memoryOf(o()) != null ? formatGpuBytes(memoryOf(o())!) : ""}
                                        </span>
                                    </div>
                                )}
                            </Show>
                        </div>
                    </Show>
                </Show>
            </div>
        </Show>
    );
}

/** How often the figures are made, and the page's own busy share, which the panel has room for. */
export function GpuFooter(props: { sample: GpuSampleResult | null; uiBusy: number | null }): JSX.Element {
    const seconds = () => Math.round((props.sample?.interval_ms ?? MIN_POLL_MS) / 1000);
    return (
        <div class="gpu-footer">
            <Show when={props.sample?.state === "ok"}>
                <span class="gpu-footer-note" classList={{ "gpu-footer-widened": !!props.sample?.widened }}>
                    {props.sample?.widened
                        ? t("gpuPanel.footer.widened", { seconds: seconds(), ms: props.sample?.sample_ms ?? 0 })
                        : t("gpuPanel.footer.every", { seconds: seconds() })}
                </span>
            </Show>
            <span
                class="gpu-footer-ui"
                style={{ color: props.uiBusy != null ? uiBusyColor(props.uiBusy) : undefined }}
                {...tip(t("uiBusy.tip", { warn: UI_BUSY_WARN_PCT }))}
            >
                {props.uiBusy != null ? t("uiBusy.value", { pct: props.uiBusy }) : t("uiBusy.hidden")}
            </span>
        </div>
    );
}
