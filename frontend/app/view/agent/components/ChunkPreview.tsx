// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ChunkPreview — streamed output, as it arrives: a running tool's log, a
 * persistent shell's log, an activity-dock row.
 *
 * The chunks are cut wherever the stream flushed, often mid-line; they used
 * to render one block each, so a line could break mid-way while streaming and
 * join up when the tool finished. They are now joined back into lines by the
 * preview text stage (`preview-text/docs.ts` `chunksDoc`) and drawn by
 * `PreviewLines`, the same as the finished output
 * (docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §3.3, §5.2).
 *
 * The order matters (#2330): drop bashwrap's starting notice, collapse
 * spinner redraws over the raw append-only stream, THEN window to the line
 * budget (createChunkCapper), and only then join — so a long stream costs the
 * last 1000 lines' worth of work per update, not the whole stream's.
 */

import { createMemo, type JSX } from "solid-js";
import { chunksDoc, type OutputChunk } from "../preview-text/docs";
import {
    createChunkCapper,
    createSpinnerCollapser,
    dropBashwrapStartingChunk,
    MAX_TOOL_OUTPUT_LINES,
} from "./output-cap";
import { PreviewLines } from "./PreviewLines";

interface ChunkPreviewProps {
    chunks: ReadonlyArray<OutputChunk>;
    /** Turn URLs into links (a shell's log). */
    linkify?: boolean;
    class?: string;
}

export function ChunkPreview(props: ChunkPreviewProps): JSX.Element {
    // Both stateful (append-only identity tracking), so one each per mounted preview.
    const spinnerCollapse = createSpinnerCollapser<OutputChunk>();
    const window = createChunkCapper(MAX_TOOL_OUTPUT_LINES);
    // Chunks seen in the stream, added as they arrive (the stream only grows;
    // a new stream starts over), so telling them apart from the collapser's
    // own frames costs the new chunks, not the whole stream.
    let fromStream = new WeakSet<OutputChunk>();
    let seen = 0;
    let first: OutputChunk | undefined;
    const doc = createMemo(() => {
        const raw = dropBashwrapStartingChunk(props.chunks);
        if (raw[0] !== first || raw.length < seen) {
            fromStream = new WeakSet();
            seen = 0;
            first = raw[0];
        }
        for (; seen < raw.length; seen++) fromStream.add(raw[seen]);
        const { display, spinnerSlot } = spinnerCollapse(raw);
        const { chunks, hiddenLines } = window(display);
        // The collapser hands back frozen frames and the live spinner trimmed,
        // without their line break: each stands as a line of its own. Those
        // are the chunks it made itself, not ones from the stream.
        const wholeLines = new Set<OutputChunk>(chunks.filter((c) => !fromStream.has(c)));
        if (spinnerSlot) wholeLines.add(spinnerSlot);
        const doc = chunksDoc(spinnerSlot ? [...chunks, spinnerSlot] : chunks, { wholeLines });
        const hidden = hiddenLines + (doc.hidden?.count ?? 0);
        return hidden > 0 ? { ...doc, hidden: { count: hidden, from: "tail" as const } } : doc;
    });
    return <PreviewLines doc={doc()} linkify={props.linkify} class={props.class} />;
}
