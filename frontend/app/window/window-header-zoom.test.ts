// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The window header's width under chrome zoom differs by engine, and these
// rules have regressed each time one platform's fix was copied to another
// (docs/retro/retro-chrome-zoom-*.md). Pin them:
//
//   - Windows and Linux run Chromium (WebView2/CEF), where `zoom` scales the
//     header's own width: it must be `calc(100vw / var(--zoomfactor, 1))`, or
//     the right-side widgets drift left when zoomed out (and off-screen when
//     zoomed in). A plain `100vw` was right only on Linux's old WebKitGTK.
//   - macOS (WebKit) must use `width: 100%`: the calc form double-divides.

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const scss = (platform: string) => readFileSync(resolve(here, `window-header.${platform}.scss`), "utf8");

/** The `width:` declarations of the `.window-header` rule blocks. */
const headerWidths = (src: string) =>
    [...src.matchAll(/^\.window-header \{([\s\S]*?)^\}/gm)].flatMap((m) =>
        [...m[1].matchAll(/^ {4}width:\s*([^;]+);/gm)].map((w) => w[1].trim())
    );

describe("window header width under chrome zoom", () => {
    it.each(["win32", "linux"])("%s (Chromium) divides 100vw by the zoom", (platform) => {
        expect(headerWidths(scss(platform))).toEqual(["calc(100vw / var(--zoomfactor, 1))"]);
    });

    it("darwin (WebKit) fills its parent instead", () => {
        expect(headerWidths(scss("darwin"))).toEqual(["100%"]);
    });
});
