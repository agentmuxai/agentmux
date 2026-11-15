// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's hex viewer: any file as rows of an offset, its bytes in hex
 * and their text, read 64 KB at a time so a file of any size opens at once.
 * A cursor and a selection (keys, click and drag), go to an offset, 8, 16 or
 * 32 bytes to a row, copy as hex, a C array, base64 or text, and a data
 * inspector for the bytes at the cursor. Read-only.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

import { Button, IconButton, MenuTrigger, SegmentedControl, type MenuEntry } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { keyLabel } from "@/app/keybindings";
import { For, Show, type JSX } from "solid-js";
import { ViewerBar } from "../viewer-bar";
import { HexFindBar } from "./hex-find-bar";
import { contentHeight, hexOf, offsetText, ROW_H, ROW_WIDTHS, type RowWidth } from "./hex-layout";
import { createHexViewer, type HexViewerProps, type HexViewerState } from "./hex-viewer-state";
import "./hex-viewer.scss";

export type { HexViewerProps } from "./hex-viewer-state";

export function HexViewer(props: HexViewerProps): JSX.Element {
    const t = createHexViewer(props);
    return (
        <div class="editor-viewer hex-viewer" style={{ "--hex-row-h": `${ROW_H}px` }} onKeyDown={t.onToolbarKeyDown}>
            <ViewerBar {...props.bar} facts={t.facts()} start={<HexTools t={t} />} />
            <Show when={t.error()}>
                <div class="hex-viewer-error" role="alert">
                    <span>{t.error()}</span>
                    <Button density="compact" onClick={t.retry}>
                        Try again
                    </Button>
                </div>
            </Show>
            <Show when={t.find.open()}>
                <HexFindBar find={t.find} />
            </Show>
            <div class="hex-viewer-body">
                <Show when={t.size() > 0} fallback={<div class="editor-viewer-message">This file is empty.</div>}>
                    <HexGrid t={t} />
                    <Show when={t.inspector()}>
                        <HexInspector t={t} />
                    </Show>
                </Show>
            </div>
        </div>
    );
}

/** The hex viewer's own controls in the viewer bar. */
function HexTools(props: { t: HexViewerState }): JSX.Element {
    const t = props.t;
    const copyItems = (): MenuEntry[] => [
        { label: "Copy as hex", shortcut: keyLabel("mod+c"), onSelect: () => void t.copy("hex") },
        { label: "Copy as a C array", onSelect: () => void t.copy("c") },
        { label: "Copy as base64", onSelect: () => void t.copy("base64") },
        { label: "Copy as text", onSelect: () => void t.copy("text") },
        { label: "Copy the offset", onSelect: t.copyOffset },
    ];
    return (
        <>
            <input
                ref={t.bindOffsetInput}
                class="hex-viewer-offset-input"
                data-raw-text-field="the viewer bar's offset box: a short number beside the other bar controls, with its own mod+g focus and Enter to go, not a form field"
                aria-label="Go to offset"
                aria-invalid={t.badOffset()}
                {...tip(`Go to an offset (${keyLabel("mod+g")}): decimal, or hex as 0x1F`)}
                value={`0x${offsetText(t.cursor(), 1)}`}
                onInput={t.clearBadOffset}
                onKeyDown={(e) => e.key === "Enter" && t.goToTyped(e.currentTarget.value)}
            />
            <SegmentedControl<string>
                density="compact"
                ariaLabel="Bytes in a row"
                options={ROW_WIDTHS.map((w) => ({ value: String(w), label: String(w), title: `${w} bytes in a row` }))}
                value={String(t.width())}
                onChange={(v) => t.setRowWidth(Number(v) as RowWidth)}
            />
            <MenuTrigger items={copyItems()} aria-label="Copy" placement="bottom-start">
                <IconButton icon="copy" density="compact" label="Copy the selection…" disabled={t.size() === 0} aria-haspopup="menu" />
            </MenuTrigger>
            <IconButton icon="table-list" density="compact" pressed={t.inspector()} label="Data inspector (I)" onClick={t.toggleInspector} />
        </>
    );
}

/** The rows: a header, then only the rows on screen, placed in a spacer as tall as the file's rows. */
function HexGrid(props: { t: HexViewerState }): JSX.Element {
    const t = props.t;
    const columns = () => Array.from({ length: t.width() }, (_, i) => i);
    return (
        <div
            ref={t.bindScroller}
            class="hex-grid"
            role="grid"
            tabIndex={0}
            aria-label="The file's bytes"
            aria-readonly="true"
            aria-rowcount={t.rows() + 1}
            aria-activedescendant={t.cellId(t.cursor())}
            data-column={t.column()}
            style={{ "--hex-digits": t.digits(), "--hex-width": t.width() }}
            onScroll={t.onScroll}
            onKeyDown={t.onKeyDown}
            onPointerDown={t.onPointerDown}
            onPointerMove={t.onPointerMove}
        >
            {/* As wide as a row, so the header and the rows scroll sideways together. */}
            <div class="hex-sheet" role="presentation">
                <div class="hex-row hex-head" role="row" aria-rowindex={1}>
                    <span class="hex-offset" role="columnheader">
                        Offset
                    </span>
                    <span class="hex-bytes" role="columnheader" aria-label="Bytes">
                        <For each={columns()}>{(i) => <span>{hexOf(i)}</span>}</For>
                    </span>
                    <span class="hex-text" role="columnheader">
                        Text
                    </span>
                </div>
                <div class="hex-spacer" role="presentation" style={{ height: `${contentHeight(t.rows())}px` }}>
                    <div class="hex-rows" role="presentation" style={{ "margin-top": `${t.layerTop()}px` }}>
                        <For each={t.shownRows()}>{(row) => <HexRow t={t} row={row} />}</For>
                    </div>
                </div>
            </div>
        </div>
    );
}

function HexRow(props: { t: HexViewerState; row: number }): JSX.Element {
    const t = props.t;
    const start = () => props.row * t.width();
    const offsets = () => Array.from({ length: Math.max(0, Math.min(t.width(), t.size() - start())) }, (_, i) => start() + i);
    return (
        <div class="hex-row" role="row" aria-rowindex={props.row + 2}>
            <span class="hex-offset" role="rowheader">
                {offsetText(start(), t.digits())}
            </span>
            <span class="hex-bytes" data-c="hex" role="presentation">
                <For each={offsets()}>
                    {(o) => (
                        <span role="gridcell" id={t.cellId(o)} data-o={o} aria-selected={t.isSelected(o)} classList={{ "hex-sel": t.isSelected(o), "hex-cur": t.cursor() === o }}>
                            {t.hexAt(o)}
                        </span>
                    )}
                </For>
            </span>
            <span class="hex-text" data-c="text" role="gridcell">
                <For each={offsets()}>
                    {(o) => (
                        <span data-o={o} classList={{ "hex-sel": t.isSelected(o), "hex-cur": t.cursor() === o }}>
                            {t.charAt(o)}
                        </span>
                    )}
                </For>
            </span>
        </div>
    );
}

/** The bytes at the cursor, as each common type. */
function HexInspector(props: { t: HexViewerState }): JSX.Element {
    const t = props.t;
    return (
        <aside class="hex-inspector" aria-label="Data inspector">
            <SegmentedControl<string>
                density="compact"
                ariaLabel="Byte order"
                options={[
                    { value: "le", label: "Little endian", title: "The lowest byte first" },
                    { value: "be", label: "Big endian", title: "The highest byte first" },
                ]}
                value={t.littleEndian() ? "le" : "be"}
                onChange={(v) => t.setLittleEndian(v === "le")}
            />
            <dl class="hex-inspector-values">
                <For each={t.inspected()}>
                    {(row) => (
                        <>
                            <dt>{row.label}</dt>
                            <dd classList={{ "hex-inspector-none": !row.value }}>{row.value || "–"}</dd>
                        </>
                    )}
                </For>
            </dl>
        </aside>
    );
}
