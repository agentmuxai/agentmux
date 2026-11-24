// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's Table viewer: a CSV or TSV file as a grid of cells, with the
 * first row as the header (a toggle). An option for a delimited file, never
 * the default. Read whole (the file is under the editor's 10 MB cap), parsed
 * here, and drawn a screenful of rows at a time. Read-only.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4 (phase 5).
 */

import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";
import { formatBytes } from "@/util/format-bytes";
import { createEffect, createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { rangeReaderOf } from "../hex/hex-chunks";
import { contentHeight, ROW_H } from "../hex/hex-layout";
import { createLineScroll } from "../large/line-scroll";
import { keyTarget } from "../large/large-text-state";
import { decoderLabel } from "../large/line-source";
import { ViewerBar, type ViewerBarProps } from "../viewer-bar";
import type { FileProbe } from "../viewer-choice";
import { columnCount, columnWidths, DELIMITER_NAMES, parseDelimited, sniffDelimiter, type Delimiter } from "./delimited";
import "./table-viewer.scss";

export interface TableViewerProps {
    path: string;
    /** Bumped when the file changed on disk: read it again. */
    revision: number;
    probe: FileProbe | null;
    blockId?: string;
    bar: Omit<ViewerBarProps, "start" | "facts">;
}

interface Parsed {
    rows: string[][];
    delimiter: Delimiter;
}

/** The delimiter the file's type says, when it says. */
function hintOf(probe: FileProbe | null): Delimiter | undefined {
    return probe?.mime === "text/tab-separated-values" ? "\t" : undefined;
}

/** The file's cells: read, decoded and parsed again when the path or the file changes. */
function createTable(props: TableViewerProps) {
    const [parsed, setParsed] = createSignal<Parsed | null>(null);
    const [error, setError] = createSignal("");
    let token = 0;
    createEffect(() => {
        const { path, probe } = props;
        void props.revision;
        const mine = ++token;
        setError("");
        if (!path) {
            setParsed(null);
            return;
        }
        void (async () => {
            try {
                const size = probe?.size ?? 0;
                const bytes = size > 0 ? await rangeReaderOf(path, props.blockId)(0, size - 1) : new Uint8Array();
                if (mine !== token) return;
                const text = new TextDecoder(decoderLabel(probe?.text?.encoding)).decode(bytes);
                const delimiter = sniffDelimiter(text, hintOf(probe));
                setParsed({ rows: parseDelimited(text, delimiter), delimiter });
            } catch (e) {
                if (mine === token) setError(`Couldn't read the file: ${e instanceof Error ? e.message : String(e)}`);
            }
        })();
    });
    onCleanup(() => token++);
    return { parsed, error };
}

export function TableViewer(props: TableViewerProps): JSX.Element {
    const { parsed, error } = createTable(props);
    const [header, setHeader] = createSignal(true);
    const all = () => parsed()?.rows ?? [];
    const head = () => (header() ? (all()[0] ?? null) : null);
    const body = createMemo(() => (header() ? all().slice(1) : all()));
    const columns = createMemo(() => columnCount(all()));
    const widths = createMemo(() => columnWidths(all(), columns()));
    const scroll = createLineScroll(() => body().length);
    const digits = () => Math.max(3, String(body().length).length);
    const facts = () => {
        const p = parsed();
        if (!p) return [];
        const count = (n: number, one: string) => `${n.toLocaleString("en-US")} ${n === 1 ? one : `${one}s`}`;
        const out = [count(body().length, "row"), count(columns(), "column"), DELIMITER_NAMES[p.delimiter]];
        if (props.probe) out.push(formatBytes(props.probe.size));
        return out;
    };
    const onKeyDown = (e: KeyboardEvent): void => {
        if (e.altKey) return;
        const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
        const to = keyTarget(key, e.ctrlKey || e.metaKey, { first: scroll.first(), fits: scroll.fits(), total: body().length });
        if (typeof to !== "number") return;
        e.preventDefault();
        e.stopPropagation();
        scroll.showLine(to);
    };
    return (
        <div class="editor-viewer table-viewer" style={{ "--table-row-h": `${ROW_H}px` }}>
            <ViewerBar
                {...props.bar}
                facts={facts()}
                start={
                    <Button
                        density="compact"
                        pressed={header()}
                        aria-label={t("editor.table.header")}
                        {...tip(t("editor.table.headerTip"))}
                        onClick={() => setHeader((v) => !v)}
                    >
                        {t("editor.table.header")}
                    </Button>
                }
            />
            <Show when={error()}>
                <div class="table-viewer-error" role="alert">
                    {error()}
                </div>
            </Show>
            <div class="table-viewer-body">
                <Show when={parsed()} fallback={<Show when={!error()}>{<div class="editor-viewer-message">{t("editor.table.reading")}</div>}</Show>}>
                    <Show when={all().length > 0} fallback={<div class="editor-viewer-message">{t("editor.table.empty")}</div>}>
                        <div
                            ref={scroll.bind}
                            class="table-viewer-grid"
                            role="grid"
                            aria-label={t("editor.table.gridLabel")}
                            aria-rowcount={all().length}
                            aria-colcount={columns()}
                            tabIndex={0}
                            style={{ "--table-digits": digits() }}
                            onScroll={scroll.onScroll}
                            onKeyDown={onKeyDown}
                        >
                            <Show when={head()}>{(cells) => <Row cells={cells()} widths={widths()} columns={columns()} number="" header />}</Show>
                            <div class="table-viewer-spacer" style={{ height: `${contentHeight(body().length)}px` }}>
                                <div class="table-viewer-rows" style={{ "margin-top": `${scroll.layerTop()}px` }}>
                                    <For each={scroll.shown()}>
                                        {(n) => <Row cells={body()[n] ?? []} widths={widths()} columns={columns()} number={String(n + 1)} />}
                                    </For>
                                </div>
                            </div>
                        </div>
                    </Show>
                </Show>
            </div>
        </div>
    );
}

function Row(props: { cells: string[]; widths: number[]; columns: number; number: string; header?: boolean }): JSX.Element {
    return (
        <div class="table-viewer-row" classList={{ "table-viewer-head": props.header }} role="row">
            <span class="table-viewer-number" aria-hidden="true">
                {props.number}
            </span>
            <For each={props.widths.slice(0, props.columns)}>
                {(w, c) => (
                    <span class="table-viewer-cell" role={props.header ? "columnheader" : "gridcell"} style={{ width: `${w + 2}ch` }} title={props.cells[c()]}>
                        {props.cells[c()] ?? ""}
                    </span>
                )}
            </For>
        </div>
    );
}
