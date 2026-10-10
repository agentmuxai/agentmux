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

/**
 * Nerd Font icons live in the Unicode Private Use Areas. Each of the four Hack
 * Nerd Mono files used to carry the same 10,071 icon glyphs (identical
 * outlines and advances in all four), about 3 MB of the 4.2 MB total, loaded
 * eagerly four times over (#4207 F2). The four files are now text-only, and the
 * icons are one shared file.
 *
 * Registering that file under the `Hack` family itself, once per weight/style
 * with a `unicode-range`, keeps every existing font stack working unchanged:
 * the browser picks the face by code point, and all four registrations share a
 * URL, so it is fetched once. The two ranges don't overlap, so which face
 * draws a code point never depends on registration order.
 *
 * The files are subsets of the previous ones (fonttools pyftsubset), with every
 * glyph outline, advance, hinting and layout table unchanged.
 */
const HACK_TEXT_RANGE = "U+0000-DFFF, U+F900-EFFFF";
const HACK_ICON_RANGE = "U+E000-F8FF, U+F0000-10FFFF";
const HACK_FACES: { file: string; style: string; weight: string; label: string }[] = [
    { file: "hacknerdmono-regular", style: "normal", weight: "400", label: "Hack Regular" },
    { file: "hacknerdmono-bold", style: "normal", weight: "700", label: "Hack Bold" },
    { file: "hacknerdmono-italic", style: "italic", weight: "400", label: "Hack Italic" },
    { file: "hacknerdmono-bolditalic", style: "italic", weight: "700", label: "Hack BoldItalic" },
];

function loadHackNerdFont() {
    if (isHackNerdFontLoaded) {
        return;
    }
    isHackNerdFontLoaded = true;
    for (const { file, style, weight, label } of HACK_FACES) {
        const text = new FontFace("Hack", `url('/fonts/${file}.woff2')`, {
            style,
            weight,
            unicodeRange: HACK_TEXT_RANGE,
        });
        const icons = new FontFace("Hack", "url('/fonts/hacknerdmono-symbols.woff2')", {
            style,
            weight,
            unicodeRange: HACK_ICON_RANGE,
        });
        addToFontFaceSet(document.fonts, text);
        addToFontFaceSet(document.fonts, icons);
        loadAndLog(text, label);
        // Same URL for all four: the first load fetches it, the rest resolve
        // from that request. Loaded eagerly like the text faces, so a prompt's
        // first icon doesn't wait on a fetch.
        loadAndLog(icons, `${label} (Nerd Font icons)`);
    }
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
