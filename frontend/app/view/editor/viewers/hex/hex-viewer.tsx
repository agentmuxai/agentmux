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
import { t } from "@/app/i18n/i18n";
import { keyLabel } from "@/app/keybindings";
import { For, Show, type JSX } from "solid-js";
import { ViewerBar } from "../viewer-bar";
import { HexFindBar } from "./hex-find-bar";
import { contentHeight, hexOf, offsetText, ROW_H, ROW_WIDTHS, type RowWidth } from "./hex-layout";
import { createHexViewer, type HexViewerProps, type HexViewerState } from "./hex-viewer-state";
import "./hex-viewer.scss";

export type { HexViewerProps } from "./hex-viewer-state";

export function HexViewer(props: HexViewerProps): JSX.Element {
    const hv = createHexViewer(props);
    return (
        <div class="editor-viewer hex-viewer" style={{ "--hex-row-h": `${ROW_H}px` }} onKeyDown={hv.onToolbarKeyDown}>
            <ViewerBar {...props.bar} facts={hv.facts()} start={<HexTools t={hv} />} />
            <Show when={hv.error()}>
                <div class="hex-viewer-error" role="alert">
                    <span>{hv.error()}</span>
                    <Button density="compact" onClick={hv.retry}>
                        {t("hexViewer.retry")}
                    </Button>
                </div>
            </Show>
            <Show when={hv.find.open()}>
                <HexFindBar find={hv.find} />
            </Show>
            <div class="hex-viewer-body">
                <Show when={hv.size() > 0} fallback={<div class="editor-viewer-message">{t("hexViewer.empty")}</div>}>
                    <HexGrid t={hv} />
                    <Show when={hv.inspector()}>
                        <HexInspector t={hv} />
                    </Show>
                </Show>
            </div>
        </div>
    );
}

/** The hex viewer's own controls in the viewer bar. */
function HexTools(props: { t: HexViewerState }): JSX.Element {
    const hv = props.t;
    const copyItems = (): MenuEntry[] => [
        { label: t("hexViewer.copyHex"), shortcut: keyLabel("mod+c"), onSelect: () => void hv.copy("hex") },
        { label: t("hexViewer.copyC"), onSelect: () => void hv.copy("c") },
        { label: t("hexViewer.copyBase64"), onSelect: () => void hv.copy("base64") },
        { label: t("hexViewer.copyText"), onSelect: () => void hv.copy("text") },
        { label: t("hexViewer.copyOffset"), onSelect: hv.copyOffset },
    ];
    return (
        <>
            <input
                ref={hv.bindOffsetInput}
                class="hex-viewer-offset-input"
                data-raw-text-field="the viewer bar's offset box: a short number beside the other bar controls, with its own mod+g focus and Enter to go, not a form field"
                aria-label={t("hexViewer.goTo")}
                aria-invalid={hv.badOffset()}
                {...tip(t("hexViewer.goToTip", { key: keyLabel("mod+g") }))}
                value={`0x${offsetText(hv.cursor(), 1)}`}
                onInput={hv.clearBadOffset}
                onKeyDown={(e) => e.key === "Enter" && hv.goToTyped(e.currentTarget.value)}
            />
            <SegmentedControl<string>
                density="compact"
                ariaLabel={t("hexViewer.rowWidth")}
                options={ROW_WIDTHS.map((w) => ({ value: String(w), label: String(w), title: t("hexViewer.rowWidthTip", { count: w }) }))}
                value={String(hv.width())}
                onChange={(v) => hv.setRowWidth(Number(v) as RowWidth)}
            />
            <MenuTrigger items={copyItems()} aria-label={t("hexViewer.copy")} placement="bottom-start">
                <IconButton icon="copy" density="compact" label={t("hexViewer.copySelection")} disabled={hv.size() === 0} aria-haspopup="menu" />
            </MenuTrigger>
            <IconButton icon="table-list" density="compact" pressed={hv.inspector()} label={t("hexViewer.inspectorTip")} onClick={hv.toggleInspector} />
        </>
    );
}

/** The rows: a header, then only the rows on screen, placed in a spacer as tall as the file's rows. */
function HexGrid(props: { t: HexViewerState }): JSX.Element {
    const hv = props.t;
    const columns = () => Array.from({ length: hv.width() }, (_, i) => i);
    return (
        <div
            ref={hv.bindScroller}
            class="hex-grid"
            role="grid"
            tabIndex={0}
            aria-label={t("hexViewer.bytesOfFile")}
            aria-readonly="true"
            aria-rowcount={hv.rows() + 1}
            aria-activedescendant={hv.cellId(hv.cursor())}
            data-column={hv.column()}
            style={{ "--hex-digits": hv.digits(), "--hex-width": hv.width() }}
            onScroll={hv.onScroll}
            onKeyDown={hv.onKeyDown}
            onPointerDown={hv.onPointerDown}
            onPointerMove={hv.onPointerMove}
        >
            {/* As wide as a row, so the header and the rows scroll sideways together. */}
            <div class="hex-sheet" role="presentation">
                <div class="hex-row hex-head" role="row" aria-rowindex={1}>
                    <span class="hex-offset" role="columnheader">
                        {t("hexViewer.offset")}
                    </span>
                    <span class="hex-bytes" role="columnheader" aria-label={t("hexViewer.bytes")}>
                        <For each={columns()}>{(i) => <span>{hexOf(i)}</span>}</For>
                    </span>
                    <span class="hex-text" role="columnheader">
                        {t("hexViewer.text")}
                    </span>
                </div>
                <div class="hex-spacer" role="presentation" style={{ height: `${contentHeight(hv.rows())}px` }}>
                    <div class="hex-rows" role="presentation" style={{ "margin-top": `${hv.layerTop()}px` }}>
                        <For each={hv.shownRows()}>{(row) => <HexRow t={hv} row={row} />}</For>
                    </div>
                </div>
            </div>
        </div>
    );
}

function HexRow(props: { t: HexViewerState; row: number }): JSX.Element {
    const hv = props.t;
    const start = () => props.row * hv.width();
    const offsets = () => Array.from({ length: Math.max(0, Math.min(hv.width(), hv.size() - start())) }, (_, i) => start() + i);
    return (
        <div class="hex-row" role="row" aria-rowindex={props.row + 2}>
            <span class="hex-offset" role="rowheader">
                {offsetText(start(), hv.digits())}
            </span>
            <span class="hex-bytes" data-c="hex" role="presentation">
                <For each={offsets()}>
                    {(o) => (
                        <span role="gridcell" id={hv.cellId(o)} data-o={o} aria-selected={hv.isSelected(o)} classList={{ "hex-sel": hv.isSelected(o), "hex-cur": hv.cursor() === o }}>
                            {hv.hexAt(o)}
                        </span>
                    )}
                </For>
            </span>
            <span class="hex-text" data-c="text" role="gridcell">
                <For each={offsets()}>
                    {(o) => (
                        <span data-o={o} classList={{ "hex-sel": hv.isSelected(o), "hex-cur": hv.cursor() === o }}>
                            {hv.charAt(o)}
                        </span>
                    )}
                </For>
            </span>
        </div>
    );
}

/** The bytes at the cursor, as each common type. */
function HexInspector(props: { t: HexViewerState }): JSX.Element {
    const hv = props.t;
    return (
        <aside class="hex-inspector" aria-label={t("hexViewer.inspector")}>
            <SegmentedControl<string>
                density="compact"
                ariaLabel={t("hexViewer.byteOrder")}
                options={[
                    { value: "le", label: t("hexViewer.little"), title: t("hexViewer.littleTip") },
                    { value: "be", label: t("hexViewer.big"), title: t("hexViewer.bigTip") },
                ]}
                value={hv.littleEndian() ? "le" : "be"}
                onChange={(v) => hv.setLittleEndian(v === "le")}
            />
            <dl class="hex-inspector-values">
                <For each={hv.inspected()}>
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
