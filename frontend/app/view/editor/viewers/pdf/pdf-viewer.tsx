// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's PDF viewer: pdf.js's own page viewer (virtualized pages, a
 * selectable text layer, links) inside the Editor's layout, so it works in
 * splits, floating panes and every theme. Scripting, XFA and eval are off
 * (pdfjs-runtime.ts). A large file loads in ranges, so a 300 MB scan opens
 * at page 1 without downloading all of it. External links open in a Browser
 * pane, never in the app; other link schemes are dropped.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.4, §4.7.
 */

import { Button, IconButton } from "@/app/element/ui";
import { keyLabel } from "@/app/keybindings";
import { Show, type JSX } from "solid-js";
import { ViewerBar } from "../viewer-bar";
import { facts, find, goTo, onKeyDown } from "./pdf-viewer-actions";
import { PdfViewerBody } from "./pdf-viewer-body";
import { usePdfLoading, usePdfZoom } from "./pdf-viewer-effects";
import type { PdfViewerProps } from "./pdf-viewer-model";
import { createPdfViewerState, type PdfViewerState } from "./pdf-viewer-state";
import { zoomBy } from "./pdf-viewer-zoom";
import "./pdf-viewer.scss";

export { WHOLE_FILE_MAX_BYTES, type PdfViewerProps } from "./pdf-viewer-model";

export function PdfViewer(props: PdfViewerProps): JSX.Element {
    const s = createPdfViewerState(props);
    usePdfLoading(s);
    usePdfZoom(s);

    return (
        <div class="editor-viewer pdf-viewer" onKeyDown={(e) => onKeyDown(s, e)}>
            <ViewerBar {...props.bar} facts={facts(s)} start={<PdfViewerControls s={s} />} />
            <PdfViewerBody s={s} />
        </div>
    );
}

/** The PDF viewer's own controls in the viewer bar: pages, zoom, invert and find. */
function PdfViewerControls(props: { s: PdfViewerState }): JSX.Element {
    const s = props.s;
    return (
        <>
            <IconButton icon="chevron-up" density="compact" label="Previous page" disabled={s.page() <= 1} onClick={() => goTo(s, s.page() - 1)} />
            <input
                ref={(el) => (s.pageInput = el)}
                class="pdf-viewer-page-input" data-raw-text-field="the viewer bar's page number: a few digits between the arrows, with its own mod+g focus and Enter to go, not a form field"
                aria-label="Page"
                title={`Page (${keyLabel("mod+g")})`}
                value={s.page()}
                onChange={(e) => goTo(s, Number(e.currentTarget.value) || s.page())}
                onKeyDown={(e) => e.key === "Enter" && goTo(s, Number(e.currentTarget.value) || s.page())}
            />
            <span class="pdf-viewer-of">/ {s.pages() || "–"}</span>
            <IconButton icon="chevron-down" density="compact" label="Next page" disabled={s.page() >= s.pages()} onClick={() => goTo(s, s.page() + 1)} />
            <IconButton icon="magnifying-glass-minus" density="compact" label={`Zoom out (${keyLabel("mod+-")})`} onClick={() => zoomBy(s, "out")} />
            <span class="pdf-viewer-scale" aria-label="Zoom">
                {Math.round(s.scale() * 100)}%
            </span>
            <IconButton icon="magnifying-glass-plus" density="compact" label={`Zoom in (${keyLabel("mod+=")})`} onClick={() => zoomBy(s, "in")} />
            <Button density="compact" title="Fit the page width" onClick={() => zoomBy(s, "page-width")}>
                Width
            </Button>
            <Button density="compact" title="Fit the whole page" onClick={() => zoomBy(s, "page-fit")}>
                Page
            </Button>
            <IconButton
                icon="circle-half-stroke"
                density="compact"
                pressed={s.invert()}
                label={s.invert() ? "Show the pages as they are" : "Invert the pages (for a dark theme)"}
                onClick={() => s.setInvert(!s.invert())}
            />
            <input
                ref={(el) => (s.findInput = el)}
                class="pdf-viewer-find" data-raw-text-field="the viewer bar's find box: it sits in the bar's row with its own mod+f focus and Enter for the next match, not a form field"
                type="search"
                placeholder="Find"
                aria-label="Find in the document"
                title={`Find (${keyLabel("mod+f")}); Enter for the next match, Shift+Enter for the previous`}
                value={s.query()}
                onInput={(e) => {
                    s.setQuery(e.currentTarget.value);
                    find(s, false);
                }}
                onKeyDown={(e) => {
                    if (e.key === "Enter") {
                        e.preventDefault();
                        find(s, true, e.shiftKey);
                    }
                }}
            />
            <Show when={s.query() && s.matches()}>
                <span class="pdf-viewer-matches" role="status">
                    {s.matches()!.total ? `${s.matches()!.current} of ${s.matches()!.total}` : "No matches"}
                </span>
            </Show>
        </>
    );
}
