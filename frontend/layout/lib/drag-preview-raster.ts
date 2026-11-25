// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The drag ghost's picture: `toPng` of the pane's preview element, with the page's web fonts embedded once.
 *
 * `toPng` embeds every `@font-face` of the document on each call: it fetches each font file and turns it into a data
 * URL through a `FileReader` (`getWebFontCSS`). That is most of the 78 to 280 ms the main thread spent per picture
 * (the long frames named `import.then` and `FileReader.onloadend` in the field log), and it landed inside whatever click,
 * drop or drag was under way. The embedded CSS is the same for every picture, so it is computed once, at idle after the
 * first pane mounts, and handed to every `toPng` after as `fontEmbedCSS`.
 */

type Raster = typeof import("html-to-image");
type RasterOptions = NonNullable<Parameters<Raster["toPng"]>[1]>;

/** The embedded font CSS, with how many font faces the document had when it was made: a new face makes a new one. */
let fonts: { faces: number; css: Promise<string | undefined> } | undefined;
let warming = false;

const faceCount = (): number => (typeof document !== "undefined" ? (document.fonts?.size ?? 0) : 0);

function fontCss(lib: Raster, node: HTMLElement, options: RasterOptions): Promise<string | undefined> {
    const faces = faceCount();
    if (fonts && fonts.faces === faces) return fonts.css;
    const css = Promise.resolve().then(() => lib.getFontEmbedCSS(node, options)).catch(() => {
        // Not cached when it failed: the next picture tries again, and this one embeds as `toPng` always did.
        if (fonts?.css === css) fonts = undefined;
        return undefined;
    });
    fonts = { faces, css };
    return css;
}

/** The picture of `node`, as a data URL. */
export async function rasterisePane(node: HTMLElement, options: RasterOptions): Promise<string> {
    const lib = await import("html-to-image");
    const fontEmbedCSS = await fontCss(lib, node, options);
    return lib.toPng(node, fontEmbedCSS == null ? options : { ...options, fontEmbedCSS });
}

/**
 * Load the rasteriser and embed the fonts when the page is idle, once, so the first drag preview of a window is as quick as
 * the later ones. Nothing waits on it: it is not run before the first paint. The fonts are read from the pane's own preview
 * element, never from the page: reading them walks every element under the node it is given, and the page's panes can hold
 * very long transcripts.
 */
export function warmDragPreview(previewNode: () => HTMLElement | undefined, options: RasterOptions): void {
    if (warming || typeof document === "undefined") return;
    warming = true;
    const run = () => {
        const node = previewNode();
        if (!node) {
            warming = false;
            return;
        }
        void import("html-to-image").then((lib) => fontCss(lib, node, options)).catch(() => {});
    };
    if (typeof requestIdleCallback === "function") requestIdleCallback(run, { timeout: 10000 });
    else setTimeout(run, 3000);
}

/** For tests: forget what was made and started. */
export function resetDragPreviewRaster(): void {
    fonts = undefined;
    warming = false;
}
