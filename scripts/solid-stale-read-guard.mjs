// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Stops Solid's "stale read" error from taking a window's updates down, and names the code that read.
//
// `<Show>` and `<Match>` give a child function an accessor that THROWS when it is read after the condition went
// false. The throw is a plain string, and where it lands decides the damage:
//   - in a computation with no error handler above it (a root made with no owner, or a cleanup, which Solid runs
//     before it sets up the handler), `handleError` rethrows, `runUpdates` drops every update and effect queued in
//     that flush, and the computation stays stale: it throws again in every later flush that reaches it;
//   - so one such read leaves panes unmounted and never remounted, a tab strip that moves while the body does not,
//     and a cover that never lifts.
// PR 7367's catchError at the app root covers only computations created under it. The string also has no stack,
// so nothing in the log said which read it was.
//
// This changes the two accessors so a read after the condition went false answers with the last value the accessor
// held, instead of throwing, and writes one `[solid]` warning with an `Error` made where the read happened (its
// stack starts at the accessor and goes down through the reader; the release build ships source maps, so it
// decodes to the component's own file and line). A read while the condition holds is unchanged.
/**
 * @param {string} id  a module id
 * @returns {boolean}
 */
export function isSolidCore(id) {
    return /[\\/]solid-js[\\/]dist[\\/](solid|dev)\.js$/.test(id.split("?")[0]);
}

/** Solid's one-line definition of the message: the production file and the dev file differ in the text only. */
const NARROWED_ERROR = /(const narrowedError = name => `[^`]*`) ?;/;

/** `<Show>`'s accessor: dev and production alike. */
const SHOW_ACCESSOR =
    /child\(keyed \? c : \(\) => \{\s*if \(!untrack\(condition\)\) throw narrowedError\("Show"\);\s*return conditionValue\(\);\s*\}\)/;

/** `<Match>`'s accessor. */
const MATCH_ACCESSOR =
    /child\(mp\.keyed \? conditionValue\(\) : \(\) => \{\s*if \(untrack\(switchFunc\)\(\)\?\.\[0\] !== index\) throw narrowedError\("Match"\);\s*return conditionValue\(\);\s*\}\)/;

/** What the guarded accessors call: the warning, written for the first 20 and then once in a hundred. */
const STALE_READ_HELPER = [
    "let staleReads = 0;",
    "function staleRead(name, last) {",
    "  staleReads++;",
    "  if (staleReads <= 20 || staleReads % 100 === 0) {",
    "    console.warn(`[solid] a stale read from <${name}> got its last value, not an error (${staleReads} so far); the read:`, new Error(narrowedError(name)));",
    "  }",
    "  return last;",
    "}",
].join("\n");

/**
 * @param {string} code  the source of solid-js's production build (solid.js) or its dev build (dev.js)
 * @returns {string} the same module whose `<Show>` and `<Match>` accessors answer a stale read with their last value
 * @throws when Solid's file no longer has the lines this replaces, so a Solid update fails the build
 *         instead of quietly going back to an error that takes a window's updates down
 */
export function withGuardedStaleReads(code) {
    if (!NARROWED_ERROR.test(code) || !SHOW_ACCESSOR.test(code) || !MATCH_ACCESSOR.test(code)) {
        throw new Error(
            "[solid-stale-read-guard] solid-js no longer builds the <Show> and <Match> accessors the way this plugin expects. " +
                "Check scripts/solid-stale-read-guard.mjs against the new Solid."
        );
    }
    return code
        .replace(NARROWED_ERROR, (_, definition) => `${definition};\n${STALE_READ_HELPER}`)
        .replace(
            SHOW_ACCESSOR,
            'child(keyed ? c : (() => { let last = c; return () => { if (!untrack(condition)) return staleRead("Show", last); return (last = conditionValue()); }; })())'
        )
        .replace(
            MATCH_ACCESSOR,
            'child(mp.keyed ? conditionValue() : (() => { let last = conditionValue(); return () => { if (untrack(switchFunc)()?.[0] !== index) return staleRead("Match", last); return (last = conditionValue()); }; })())'
        );
}

/** @returns {import("vite").Plugin} */
export function solidStaleReadGuard() {
    return {
        name: "solid-stale-read-guard",
        enforce: "pre",
        transform(code, id) {
            if (!isSolidCore(id)) return null;
            return { code: withGuardedStaleReads(code), map: null };
        },
    };
}
