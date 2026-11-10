// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Keeps `shiki/bundle/web` from pulling every Shiki theme into the build.
//
// `shiki-highlighter.ts` takes the web grammars from `shiki/bundle/web` and loads its two themes itself
// (it says why: `bundledThemes` shipped all ~65 as chunks). But that module also builds its own highlighter
// from `bundledThemes` when it loads, and Rolldown, unlike Rollup, keeps that code: every theme became a
// chunk, none used, and the notices plugin warned about each one with no licence section.
//
// `treeshake.propertyReadSideEffects: false` also fixes it, but it applies to the whole build and would drop
// the reads the app makes on purpose to force a layout (`el.offsetHeight;`), so this changes the one module.
/**
 * @param {string} id  a module id
 * @returns {boolean}
 */
export function isShikiWebBundle(id) {
    return /[\\/]shiki[\\/]dist[\\/]bundle-web\.mjs$/.test(id.split("?")[0]);
}

/** Shiki 4's bundle-web.mjs takes both theme tables in one import, and its export list names them. */
const THEMES_IMPORT = /import \{ bundledThemes, bundledThemesInfo \} from ["']\.\/themes\.mjs["'];/;

/**
 * @param {string} code  the source of shiki's bundle-web.mjs
 * @returns {string} the same module with empty theme tables
 * @throws when Shiki's file no longer has the line this replaces, so a Shiki update fails the build
 *         instead of quietly shipping every theme again
 */
export function withoutBundledThemes(code) {
    if (!THEMES_IMPORT.test(code)) {
        throw new Error(
            "[shiki-no-themes] shiki/dist/bundle-web.mjs no longer imports its themes the way this plugin expects. " +
                "Check scripts/shiki-no-themes.mjs against the new Shiki."
        );
    }
    return code.replace(THEMES_IMPORT, "const bundledThemes = {}; const bundledThemesInfo = [];");
}

/** @returns {import("vite").Plugin} */
export function shikiNoThemes() {
    return {
        name: "shiki-no-themes",
        enforce: "pre",
        transform(code, id) {
            if (!isShikiWebBundle(id)) return null;
            return { code: withoutBundledThemes(code), map: null };
        },
    };
}
