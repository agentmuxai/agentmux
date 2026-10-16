// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { PDF_ASSETS, pdfAsset } from "./pdf-assets";

// An empty glob only shows when a PDF needs the missing file: CJK text with no
// glyphs, a standard font drawn wrong, a scanned page that won't decode.
describe("pdf.js side files", () => {
    it("are all bundled", () => {
        expect(Object.keys(PDF_ASSETS.cMapUrl.files).length).toBeGreaterThan(100);
        expect(Object.keys(PDF_ASSETS.standardFontDataUrl.files).length).toBeGreaterThan(10);
    });

    it("are found by the names pdf.js asks for", () => {
        expect(pdfAsset("cMapUrl", "UniJIS-UCS2-H.bcmap")).toBeTypeOf("function");
        expect(pdfAsset("standardFontDataUrl", "FoxitSerif.pfb")).toBeTypeOf("function");
        expect(pdfAsset("wasmUrl", "jbig2.wasm")).toBeTypeOf("function");
        expect(pdfAsset("wasmUrl", "openjpeg.wasm")).toBeTypeOf("function");
        expect(pdfAsset("wasmUrl", "nothing.wasm")).toBeUndefined();
        expect(pdfAsset("nothingUrl", "jbig2.wasm")).toBeUndefined();
    });
});
