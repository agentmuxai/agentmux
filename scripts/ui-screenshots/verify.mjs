// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A shot's `verify`: what must be true after its `prep` before anything is
// captured — see docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §9.
// `prep` clicks by selector or text and fires without checking where it
// landed (the §7 follow-up), so a click that opened the wrong thing used to be
// captured as if it were right. With `verify`, it fails the shot instead.
//
// `verify` is either
//   - a CSS selector: an element matching it must be on screen, i.e. a hit
//     test at its centre lands in it (being in the DOM with a size isn't
//     enough: hidden window tabs stay laid out under the visible one, §8.2);
//   - an array of selectors, all of which must be on screen;
//   - or a function `(session) => true | string`, where a string is the reason
//     it failed.

/** An expression that's true when an element matching `selector` is on
 *  screen: it has a size and a hit test at its centre lands in it. */
export function onScreenExpression(selector) {
    return `(() => {
        for (const el of document.querySelectorAll(${JSON.stringify(selector)})) {
            const r = el.getBoundingClientRect();
            if (r.width === 0 || r.height === 0) continue;
            const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
            if (hit && (el === hit || el.contains(hit))) return true;
        }
        return false;
    })()`;
}

/** Throws if `shot.verify` doesn't hold. A shot without one passes. */
export async function checkVerify(session, shot) {
    const v = shot.verify;
    if (!v) return;
    if (typeof v === "function") {
        const result = await v(session);
        if (result !== true) throw new Error(`verify: ${typeof result === "string" && result ? result : "failed"}`);
        return;
    }
    for (const selector of Array.isArray(v) ? v : [v]) {
        if (!(await session.evaluate(onScreenExpression(selector)))) {
            throw new Error(`verify: ${selector} isn't on screen`);
        }
    }
}
