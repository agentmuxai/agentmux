// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// NOTE: JetBrains Mono was removed here (PR #3247). It carried `calt` rules
// that ligate runs of punctuation by substituting a BLANK glyph (`glyph00388`,
// no contours) plus a composed glyph drawn leftward. The font guards this to
// runs of 2-3, but CEF 152's shaper stopped honoring those guards and applied
// it across whole runs, so holding "." rendered as blank cells with a single
// dot at the end. Nothing ever selected this font deliberately — it only
// appeared inside fallbacks of CSS variables that are never defined — so
// dropping it removes the bug at the source rather than fighting `calt` in
// CSS (which loses to any of the app's 19 `font:` shorthands).
// See docs/retro/retro-terminal-consecutive-period-input-loss-2026-09-15.md.
//
// Hack (our actual mono default) has no `calt` table at all, so it is
// unaffected. Do not reintroduce a ligature font without first re-checking
// this on the shipped CEF milestone.
let isHackNerdFontLoaded = false;
let isInterFontLoaded = false;

function addToFontFaceSet(fontFaceSet: FontFaceSet, fontFace: FontFace) {
    // any cast to work around typing issue
    (fontFaceSet as any).add(fontFace);
}

function loadAndLog(fontFace: FontFace, label: string) {
    fontFace.load().then(
        () => console.log(`[font] loaded: ${label}`),
        (err) => console.error(`[font] FAILED to load: ${label}`, err)
    );
}

function loadHackNerdFont() {
    if (isHackNerdFontLoaded) {
        return;
    }
    isHackNerdFontLoaded = true;
    const hackRegular = new FontFace("Hack", "url('/fonts/hacknerdmono-regular.woff2')", {
        style: "normal",
        weight: "400",
    });
    const hackBold = new FontFace("Hack", "url('/fonts/hacknerdmono-bold.woff2')", {
        style: "normal",
        weight: "700",
    });
    const hackItalic = new FontFace("Hack", "url('/fonts/hacknerdmono-italic.woff2')", {
        style: "italic",
        weight: "400",
    });
    const hackBoldItalic = new FontFace("Hack", "url('/fonts/hacknerdmono-bolditalic.woff2')", {
        style: "italic",
        weight: "700",
    });
    addToFontFaceSet(document.fonts, hackRegular);
    addToFontFaceSet(document.fonts, hackBold);
    addToFontFaceSet(document.fonts, hackItalic);
    addToFontFaceSet(document.fonts, hackBoldItalic);
    loadAndLog(hackRegular, "Hack Regular");
    loadAndLog(hackBold, "Hack Bold");
    loadAndLog(hackItalic, "Hack Italic");
    loadAndLog(hackBoldItalic, "Hack BoldItalic");
}

function loadInterFont() {
    if (isInterFontLoaded) {
        return;
    }
    isInterFontLoaded = true;
    const interFont = new FontFace("Inter", "url('/fonts/inter-variable.woff2')", {
        style: "normal",
        weight: "100 900",
    });
    addToFontFaceSet(document.fonts, interFont);
    loadAndLog(interFont, "Inter Variable");
}

function loadFonts() {
    loadInterFont();
    loadHackNerdFont();
}

export { loadFonts };
