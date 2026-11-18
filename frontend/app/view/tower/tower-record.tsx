// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Memory view's "over days" panel: the record srv keeps on this machine (an hourly rollup, kept a year) as a
// line over the last day, week or month, with what the pages hold and the restarts and stalls beside it
// (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §6, task #47820). It reads
// `GET /api/v1/app/record` when it is shown and again every ten minutes; nothing is sent anywhere.

import { SegmentedControl, Select } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { authHeaders } from "@/app/store/auth-headers";
import { getWebServerEndpoint } from "@/util/endpoints";
import { createMemo, createSignal, For, type JSX, onCleanup, Show } from "solid-js";
import { Chart } from "./tower-history";
import {
    floorOf,
    formatLong,
    formatSpan,
    isPageKey,
    LONG_KEYS,
    type LongKey,
    type LongSpan,
    latestPages,
    pageWindow,
    pointsOf,
    type RecordAnswer,
    recordQuery,
    restartsIn,
    type RollupLine,
    SPAN_MS,
    type StallLine,
    stallCounts,
} from "./tower-record-data";

const REFRESH_MS = 10 * 60_000;

async function read<T>(query: string): Promise<RecordAnswer<T>> {
    const res = await fetch(`${getWebServerEndpoint()}/api/v1/app/record?${query}`, { headers: authHeaders() });
    if (!res.ok) throw new Error(`record: ${res.status}`);
    const body = (await res.json()) as { matched?: number; lines?: T[] };
    const lines = body.lines ?? [];
    return { matched: body.matched ?? lines.length, lines };
}

interface Loaded {
    rolls: RecordAnswer<RollupLine>;
    stalls: RecordAnswer<StallLine>;
}

const atWords = (ms: number) =>
    new Date(ms).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

/** The record for the chosen span: read when asked for and every ten minutes; an answer that comes after a newer
 *  question is dropped. */
function createLongRecord(now: () => number) {
    const [span, setSpan] = createSignal<LongSpan>("7d");
    const [loaded, setLoaded] = createSignal<Loaded | null>(null);
    const [failed, setFailed] = createSignal(false);
    let generation = 0;
    const load = async (s: LongSpan) => {
        const mine = ++generation;
        try {
            const [rolls, stalls] = await Promise.all([
                read<RollupLine>(recordQuery("rollup", s, now())),
                read<StallLine>(recordQuery("events", s, now())),
            ]);
            if (mine !== generation) return;
            setFailed(false);
            setLoaded({ rolls, stalls });
        } catch {
            if (mine === generation) setFailed(true);
        }
    };
    const choose = (s: LongSpan) => {
        setSpan(s);
        void load(s);
    };
    void load(span());
    const timer = setInterval(() => void load(span()), REFRESH_MS);
    onCleanup(() => {
        generation++;
        clearInterval(timer);
    });
    return { span, loaded, failed, choose };
}

const spanWords = (s: LongSpan) => t(`towerLong.span.${s}`);
const keyWords = (k: LongKey) => t(`towerLong.key.${k}`);

/** "2 UI stalls (longest 55 s)", "0 srv stalls". */
function stallWords(kind: "ui" | "srv", c: { count: number; longestMs: number }): string {
    return c.count
        ? t(`towerLong.${kind}StallLongest`, { count: c.count, span: formatSpan(c.longestMs) })
        : t(`towerLong.${kind}Stall`, { count: 0 });
}

function LongFacts(props: { lines: RollupLine[]; stalls: RecordAnswer<StallLine> }): JSX.Element {
    const counts = createMemo(() => stallCounts(props.stalls));
    const stalls = () =>
        t("towerLong.stalls", { ui: stallWords("ui", counts().ui), srv: stallWords("srv", counts().srv) });
    return (
        <div class="tower-memory-long-facts tower-muted" data-testid="tower-memory-long-facts">
            <span data-testid="tower-memory-long-restarts">
                {t("towerLong.restarts", { count: restartsIn(props.lines) })}
            </span>
            <span data-testid="tower-memory-long-stalls">
                {counts().cut ? t("towerLong.newestStalls", { stalls: stalls() }) : stalls()}
            </span>
        </div>
    );
}

function LongPages(props: { lines: RollupLine[]; chosen: LongKey }): JSX.Element {
    const pages = createMemo(() => latestPages(props.lines));
    return (
        <For each={pages()}>
            {(p) => (
                <div class="tower-memory-long-page tower-muted" data-testid="tower-memory-long-page">
                    <span>{t("towerLong.page.window", { w: p.w })}</span>
                    <Show when={p.heap}>
                        {(h) => <span>{t("towerLong.page.heap", { heap: formatLong("heap", h()[1]) })}</span>}
                    </Show>
                    <span>{t("towerLong.page.elements", { n: formatLong("dom", p.dom[1]) })}</span>
                    <span>{t("towerLong.page.rows", { n: formatLong("rows", p.rows[1]) })}</span>
                    <Show when={p.w === pageWindow(props.lines) && isPageKey(props.chosen)}>
                        <span>{t("towerLong.page.charted")}</span>
                    </Show>
                </div>
            )}
        </For>
    );
}

export function LongRun(props: { now?: () => number }): JSX.Element {
    const { span, loaded, failed, choose } = createLongRecord(props.now ?? Date.now);
    const [key, setKey] = createSignal<LongKey>("am");
    const lines = () => loaded()?.rolls.lines ?? [];
    const points = createMemo(() => pointsOf(lines(), key()));
    const note = () => (failed() ? "towerLong.failed" : loaded() ? "towerLong.fillsIn" : "towerLong.reading");
    return (
        <div class="tower-memory-long" data-testid="tower-memory-long">
            <div class="tower-memory-chart-head">
                <span class="tower-label">{t("towerLong.title")}</span>
                <span class="tower-toolbar-spacer" />
                <Select
                    density="compact"
                    ariaLabel={t("towerLong.followAria")}
                    options={LONG_KEYS.map((v) => ({ value: v, label: keyWords(v) }))}
                    value={key()}
                    onChange={(v) => setKey(v as LongKey)}
                />
                <SegmentedControl<LongSpan>
                    density="compact"
                    ariaLabel={t("towerLong.spanAria")}
                    options={(Object.keys(SPAN_MS) as LongSpan[]).map((s) => ({ value: s, label: s }))}
                    value={span()}
                    onChange={choose}
                />
            </div>
            <Show
                when={points().length >= 2}
                fallback={
                    <p class="tower-muted tower-memory-long-note" data-testid="tower-memory-long-empty">
                        {t(note())}
                    </p>
                }
            >
                <Chart
                    points={points()}
                    windowMs={SPAN_MS[span()]}
                    axis={{
                        start: t("towerLong.axisStart", { span: spanWords(span()) }),
                        end: t("towerLong.axisEnd"),
                        at: (when) => atWords(when),
                        describe: (latest, peak) =>
                            t("towerLong.describe", { what: keyWords(key()), span: spanWords(span()), latest, peak }),
                    }}
                    series={{
                        label: t("towerLong.seriesLabel", { what: keyWords(key()) }),
                        value: (p) => p.mem,
                        format: (v) => formatLong(key(), v),
                        floor: floorOf(key()),
                        color: "var(--sysinfo-mem-color)",
                    }}
                />
            </Show>
            <Show when={loaded()}>
                {(l) => (
                    <>
                        <LongFacts lines={lines()} stalls={l().stalls} />
                        <LongPages lines={lines()} chosen={key()} />
                    </>
                )}
            </Show>
        </div>
    );
}
