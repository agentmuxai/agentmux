// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

let isJetBrainsMonoLoaded = false;
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

function loadJetBrainsMonoFont() {
    if (isJetBrainsMonoLoaded) {
        return;
    }
    isJetBrainsMonoLoaded = true;
    const jbmFontNormal = new FontFace("JetBrains Mono", "url('/fonts/jetbrains-mono-v13-latin-regular.woff2')", {
        style: "normal",
        weight: "400",
    });
    const jbmFont200 = new FontFace("JetBrains Mono", "url('/fonts/jetbrains-mono-v13-latin-200.woff2')", {
        style: "normal",
        weight: "200",
    });
    const jbmFont700 = new FontFace("JetBrains Mono", "url('/fonts/jetbrains-mono-v13-latin-700.woff2')", {
        style: "normal",
        weight: "700",
    });
    addToFontFaceSet(document.fonts, jbmFontNormal);
    addToFontFaceSet(document.fonts, jbmFont200);
    addToFontFaceSet(document.fonts, jbmFont700);
    loadAndLog(jbmFontNormal, "JetBrains Mono 400");
    loadAndLog(jbmFont200, "JetBrains Mono 200");
    loadAndLog(jbmFont700, "JetBrains Mono 700");
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
    loadJetBrainsMonoFont();
    loadHackNerdFont();
}

export { loadFonts };
