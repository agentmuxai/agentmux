// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Several raw SVGs this app inlines (notably logo-brain.svg, whose 16-stop
// gradient illustration renders AgentMux's brand mark) define internal
// `<linearGradient id="...">`s with plain, non-unique ids and reference
// them via `fill="url(#...)"`. `innerHTML`-mounting the SAME raw markup
// more than once on one page (e.g. the Connectors → Accounts gallery tile AND
// the connected-accounts row both showing the AgentMux icon at once)
// creates duplicate ids — the browser resolves every `url(#id)` reference
// to whichever element with that id appears FIRST in the document,
// regardless of which copy defined it. Since these are userSpace-style
// gradients positioned relative to their OWN originally-inlined copy's
// geometry, every instance after the first paints with a gradient
// positioned for a shape it doesn't belong to — in practice, invisible
// (live-confirmed via CDP: a second inlined copy's path resolved its
// gradient fill to the FIRST copy's element). Live symptom: the AgentMux
// icon renders blank in Connectors → Accounts once both the tile and the
// connected row are visible together (post-`SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT`,
// both are now always in the same continuous scroll, not hidden in a
// separate scroll region).
//
// Fix: give every mounted instance's internal ids a unique suffix before
// setting innerHTML, so each copy's gradients/references only ever
// resolve within itself. Scoped to actual defined ids only (collected
// from `id="..."` attributes first) so this can't accidentally rewrite
// unrelated `#` occurrences elsewhere in the markup (e.g. hex color
// literals like `fill="#ff0000"`, which never match `id="..."`/`url(#...)`/
// `href="#..."`).
let nextSvgInstanceId = 0;

export function uniquifySvgIds(svg: string): string {
    const ids = new Set<string>();
    for (const m of svg.matchAll(/\bid="([\w-]+)"/g)) ids.add(m[1]);
    if (ids.size === 0) return svg;
    const suffix = `svg${nextSvgInstanceId++}`;
    let out = svg;
    for (const id of ids) {
        const uniq = `${id}-${suffix}`;
        // split/join instead of replaceAll — this codebase's TS lib target
        // predates ES2021, and these are literal-string searches (no regex
        // escaping needed).
        out = out.split(`id="${id}"`).join(`id="${uniq}"`);
        out = out.split(`url(#${id})`).join(`url(#${uniq})`);
        out = out.split(`href="#${id}"`).join(`href="#${uniq}"`);
    }
    return out;
}
