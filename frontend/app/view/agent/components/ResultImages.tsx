// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ResultImages — the images a tool returned, shown inline: a Read of a PNG,
 * an MCP tool's screenshot. The translator keeps them as base64
 * (`mediaResultOf`), so they render from memory with no fetch.
 *
 * Clicking an image with a known file opens it in a Media pane, the same as
 * an image in the agent's own message; one without (a screenshot) toggles
 * between fitted and full size.
 * docs/analysis/ANALYSIS_READ_TOOL_PREVIEW_2026_10_01.md §6.
 */

import { openInMediaPane } from "@/app/element/markdown-media";
import { createSignal, For, type JSX } from "solid-js";
import type { ResultImage } from "../providers/claude-translator";

/** A raster or SVG image type: anything else is not put in a data URL. */
const IMAGE_TYPE_RE = /^image\/[a-z0-9.+-]+$/i;

/** The images worth rendering: a known image type and some data. */
export function resultImagesOf(result: unknown): ResultImage[] {
    const images = (result as { images?: unknown } | null | undefined)?.images;
    if (!Array.isArray(images)) return [];
    return images.filter(
        (i): i is ResultImage =>
            !!i &&
            typeof i.mediaType === "string" &&
            IMAGE_TYPE_RE.test(i.mediaType) &&
            typeof i.data === "string" &&
            i.data !== ""
    );
}

function ResultImageView(props: { image: ResultImage; path?: string; width?: number; height?: number }): JSX.Element {
    const [full, setFull] = createSignal(false);
    return (
        <img
            class="agent-tool-image"
            classList={{ "agent-tool-image-full": full() }}
            src={`data:${props.image.mediaType};base64,${props.image.data}`}
            alt={props.path ? `the image ${props.path}` : "an image the tool returned"}
            title={props.path ? "Open in Media pane" : full() ? "Fit to the pane" : "Show at full size"}
            // The dimensions, when known, reserve the image's height before it
            // decodes, so the row doesn't grow under the reader.
            style={
                props.width && props.height
                    ? { width: `min(100%, ${props.width}px)`, "aspect-ratio": `${props.width} / ${props.height}` }
                    : undefined
            }
            onClick={() => (props.path ? openInMediaPane(props.path) : setFull((v) => !v))}
        />
    );
}

export function ResultImages(props: {
    images: ResultImage[];
    /** The file the image is, for a Read; opens it in a Media pane on click. */
    path?: string;
    /** The image's dimensions, when the result says (only for one image). */
    width?: number;
    height?: number;
}): JSX.Element {
    return (
        <div class="agent-tool-images">
            <For each={props.images}>
                {(image) => (
                    <ResultImageView
                        image={image}
                        path={props.path}
                        width={props.images.length === 1 ? props.width : undefined}
                        height={props.images.length === 1 ? props.height : undefined}
                    />
                )}
            </For>
        </div>
    );
}

ResultImages.displayName = "ResultImages";
