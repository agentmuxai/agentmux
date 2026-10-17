// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * pdf.js's side files, bundled by Vite: the CMaps for CJK text, the standard
 * fonts, and the JBIG2 and JPEG 2000 WebAssembly decoders. Each glob is
 * `exhaustive` because Vite leaves out `node_modules` paths otherwise; the
 * base here is already inside `node_modules`, so they match without it, but
 * that is Vite's detail to change. pdf-assets.test.ts checks they aren't
 * empty.
 *
 * Each glob names its extensions, so the folders' LICENSE_* files stay out:
 * Vite names an extensionless asset with a trailing dot, which Windows can't
 * open, and the Inno Setup installer build fails on it. Their texts ship in
 * the third-party notices instead (licenses/bundled/pdfjs-*.txt).
 */

export type UrlLoaders = Record<string, () => Promise<string>>;

// Vite reads glob options only as a literal written in the call.
const cmaps = import.meta.glob("/node_modules/pdfjs-dist/cmaps/*.bcmap", { query: "?url", import: "default", exhaustive: true }) as UrlLoaders;
const fonts = import.meta.glob("/node_modules/pdfjs-dist/standard_fonts/*.{pfb,ttf}", { query: "?url", import: "default", exhaustive: true }) as UrlLoaders;
const wasm = import.meta.glob("/node_modules/pdfjs-dist/wasm/*.{wasm,js}", { query: "?url", import: "default", exhaustive: true }) as UrlLoaders;

/** By the kind pdf.js asks for: the folder it means and the bundled files. */
export const PDF_ASSETS: Record<string, { dir: string; files: UrlLoaders }> = {
    cMapUrl: { dir: "cmaps", files: cmaps },
    standardFontDataUrl: { dir: "standard_fonts", files: fonts },
    wasmUrl: { dir: "wasm", files: wasm },
};

/** The bundled URL loader for one side file, or undefined. */
export function pdfAsset(kind: string, filename: string): (() => Promise<string>) | undefined {
    const entry = PDF_ASSETS[kind];
    return entry?.files[`/node_modules/pdfjs-dist/${entry.dir}/${filename}`];
}
