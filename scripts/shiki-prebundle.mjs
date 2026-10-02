// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Shiki modules the dev server must pre-bundle at startup.
//
// Why this exists. `shiki-highlighter.ts` loads one grammar per language with
// `import("shiki/langs/rust.mjs")` and so on, the first time a file in that
// language is highlighted. The Vite dev server does not find those at startup,
// so the first time a code block in a new language reaches the page it
// optimizes the grammar and then RELOADS THE WHOLE PAGE ("new dependencies
// optimized ... optimized dependencies changed. reloading"). When an agent's
// output carries code blocks in a dozen languages at once, that is one reload,
// and the window goes blank until it comes back.
//
// Listing them in `optimizeDeps.include` makes Vite bundle them up front.
//
// The list is DERIVED from shiki-highlighter.ts rather than copied into
// vite.config.ts, so adding a grammar there cannot reintroduce the reload.
// Dev-server only: a production build bundles every dynamic import ahead of
// time and never reloads.

/** The Shiki subpaths that are loaded lazily or through a deep path. */
const LAZY_SHIKI = /["'](shiki\/(?:langs\/[A-Za-z0-9_.-]+|engine\/[A-Za-z0-9_.-]+|wasm))["']/g;

/**
 * @param {string} highlighterSource  the text of shiki-highlighter.ts
 * @returns {string[]} sorted, de-duplicated module specifiers
 */
export function shikiPrebundleDeps(highlighterSource) {
    const found = new Set();
    for (const m of highlighterSource.matchAll(LAZY_SHIKI)) found.add(m[1]);
    return [...found].sort();
}
