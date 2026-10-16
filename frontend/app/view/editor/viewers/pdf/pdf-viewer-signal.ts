// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The abort signal every PDF viewer gives pdf.js's PDFViewer, shared, and
 * aborted only when the last viewer closes.
 *
 * pdf.js 6.4.299 releases a viewer's ResizeObserver and scroll listeners when
 * its signal aborts, but it also binds the document-wide text-selection
 * listeners, shared by every text layer, to the signal of whichever viewer
 * registered them first. Aborting that viewer's own signal while another PDF
 * is open leaves the other one's selection stuck and its links dead
 * (mozilla/pdf.js#22129, unfixed in a release as of 6.4.299). With one shared
 * signal those listeners live as long as any PDF is open; a closed viewer's
 * observers wait for the last one, then all go together.
 */

let controller: AbortController | null = null;
let holders = 0;

export function acquireViewerSignal(): { signal: AbortSignal; release: () => void } {
    controller ??= new AbortController();
    const mine = controller;
    holders++;
    let released = false;
    return {
        signal: mine.signal,
        release: () => {
            if (released) return;
            released = true;
            holders--;
            if (holders === 0) {
                mine.abort();
                controller = null;
            }
        },
    };
}
