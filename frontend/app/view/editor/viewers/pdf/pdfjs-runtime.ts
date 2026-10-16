// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * pdf.js for the Editor's PDF viewer, loaded on first use (pdf-viewer.tsx
 * imports this module dynamically, so text-only users never load it).
 * - The worker is pdf.js's own module worker, bundled by Vite.
 * - Its side files (CMaps for CJK text, the standard fonts, the JBIG2 and
 *   JPEG 2000 WebAssembly decoders) are bundled too and handed to pdf.js
 *   through a BinaryDataFactory, since Vite gives each a hashed name rather
 *   than a folder pdf.js could fetch from.
 * - Every document opens with scripting, XFA and eval off.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.4, §4.7.
 */

import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import "pdfjs-dist/web/pdf_viewer.css";
import { SAFE_FLAGS } from "./pdf-policy";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

type UrlLoaders = Record<string, () => Promise<string>>;
const cmaps = import.meta.glob("/node_modules/pdfjs-dist/cmaps/*.bcmap", { query: "?url", import: "default" }) as UrlLoaders;
const fonts = import.meta.glob("/node_modules/pdfjs-dist/standard_fonts/*", { query: "?url", import: "default" }) as UrlLoaders;
const wasm = import.meta.glob("/node_modules/pdfjs-dist/wasm/*.{wasm,js}", { query: "?url", import: "default" }) as UrlLoaders;

const BY_KIND: Record<string, { dir: string; files: UrlLoaders }> = {
    cMapUrl: { dir: "cmaps", files: cmaps },
    standardFontDataUrl: { dir: "standard_fonts", files: fonts },
    wasmUrl: { dir: "wasm", files: wasm },
};

/** pdf.js asks for a side file by kind and name; this finds its bundled URL. */
class BundledBinaryDataFactory {
    constructor(_urls: unknown) {}

    async fetch({ kind, filename }: { kind: string; filename: string }): Promise<Uint8Array> {
        const entry = BY_KIND[kind];
        const load = entry?.files[`/node_modules/pdfjs-dist/${entry.dir}/${filename}`];
        if (!load) throw new Error(`pdf.js asked for ${kind} ${filename}, which isn't bundled`);
        const resp = await fetch(await load());
        if (!resp.ok) throw new Error(`${filename}: ${resp.status}`);
        return new Uint8Array(await resp.arrayBuffer());
    }
}

/** What every document opens with: no JavaScript, no XFA, no eval. */
export const SAFE_DOCUMENT_OPTIONS = {
    ...SAFE_FLAGS,
    useWorkerFetch: false,
    BinaryDataFactory: BundledBinaryDataFactory,
    // Truthy so pdf.js asks the factory; the names are only labels here.
    cMapUrl: "bundled:cmaps/",
    cMapPacked: true,
    standardFontDataUrl: "bundled:standard_fonts/",
    wasmUrl: "bundled:wasm/",
    useWasm: true,
};

/** pdf.js and its viewer components. The viewer module reads the library
 *  from `globalThis.pdfjsLib`, so it is imported only after that is set. */
export async function loadPdfjs() {
    (globalThis as { pdfjsLib?: unknown }).pdfjsLib = pdfjs;
    const viewer = await import("pdfjs-dist/web/pdf_viewer.mjs");
    return { pdfjs, viewer };
}

export type PdfjsModules = Awaited<ReturnType<typeof loadPdfjs>>;
