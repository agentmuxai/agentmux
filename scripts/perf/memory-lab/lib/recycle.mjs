// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Measuring a renderer recycle (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md):
// what the window held before, what it holds after, and whether the user's
// state came back. A recycle that frees memory but loses a draft or a scroll
// position hasn't helped anyone.
//
// `armState` puts a draft in one agent's composer and scrolls another agent's
// transcript up a little, and records both; `checkState` reads them back after
// the recycle. Both work on the driver's current main-window page.

import { mainPage, takeSample } from "./sample.mjs";
import { groupSample, MB } from "./summary.mjs";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The draft text the check types: unique per run, so a stale one can't pass. */
export const draftText = (tag = Date.now().toString(36)) => `lab-draft-${tag}: keep this, it was not sent`;

/**
 * In-page: where `blockId`'s transcript is scrolled, as the first row that
 * reaches into the view and how far from the top it starts. Rows are the
 * `[data-node-id]` elements; the scroller is their nearest scrolling ancestor.
 */
const SCROLL_PROBE = (blockId) => `(() => {
  const pane = document.querySelector('[data-blockid=${JSON.stringify(blockId)}]');
  const first = pane && pane.querySelector('[data-node-id]');
  if (!first) return null;
  let sc = first;
  while (sc && !(sc.scrollHeight > sc.clientHeight + 20 && /auto|scroll/.test(getComputedStyle(sc).overflowY))) sc = sc.parentElement;
  if (!sc) return null;
  const top = sc.getBoundingClientRect().top;
  const row = [...pane.querySelectorAll('[data-node-id]')].find((e) => e.getBoundingClientRect().bottom > top + 1);
  return { scrollTop: Math.round(sc.scrollTop), range: Math.round(sc.scrollHeight - sc.clientHeight), node: row ? row.getAttribute('data-node-id') : null, offset: row ? Math.round(row.getBoundingClientRect().top - top) : null };
})()`;

/** The middle of `blockId`'s transcript on screen, where a wheel turn lands. */
const SCROLL_CENTER = (blockId) => `(() => {
  const pane = document.querySelector('[data-blockid=${JSON.stringify(blockId)}]');
  const first = pane && pane.querySelector('[data-node-id]');
  let sc = first;
  while (sc && !(sc.scrollHeight > sc.clientHeight + 20 && /auto|scroll/.test(getComputedStyle(sc).overflowY))) sc = sc.parentElement;
  if (!sc) return null;
  const r = sc.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
})()`;

/**
 * Scrolls `blockId`'s transcript up by `pixels` the way a user does, with the
 * mouse wheel: setting `scrollTop` from a script leaves the pane thinking it is
 * still following the bottom, which is not the state a user's scroll leaves.
 */
async function wheelUp(driver, blockId, pixels) {
    const at = await driver.eval(SCROLL_CENTER(blockId));
    if (!at) return false;
    const STEP = 100;
    await driver.session.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: at.x, y: at.y });
    for (let done = 0; done < pixels; done += STEP) {
        await driver.session.send("Input.dispatchMouseEvent", { type: "mouseWheel", x: at.x, y: at.y, deltaX: 0, deltaY: -STEP });
        await sleep(60);
    }
    return true;
}

const COMPOSER = (blockId) => `document.querySelector('[data-blockid=${JSON.stringify(blockId)}] textarea.agent-input')`;

/**
 * Puts a draft in the first agent's composer and scrolls the second agent's
 * transcript `scrollUp` pixels above its end. Returns what to check for.
 */
export async function armState(driver, { scrollUp = 600 } = {}) {
    const [a, b] = driver.agents;
    if (!a || !b) throw new Error("the state check needs two agents");
    const draft = draftText();
    const focused = await driver.eval(`(() => { const t = ${COMPOSER(a.blockId)}; if (!t) return false; t.focus(); return true; })()`);
    if (!focused) throw new Error(`no composer for ${a.name}: it must be in the active tab`);
    await driver.session.typeText(draft);
    await sleep(500);
    const scrolled = await wheelUp(driver, b.blockId, scrollUp);
    await sleep(800);
    return {
        draft: { blockId: a.blockId, text: draft, typed: await driver.eval(`${COMPOSER(a.blockId)}?.value ?? null`) },
        scroll: { blockId: b.blockId, scrolled, before: await driver.eval(SCROLL_PROBE(b.blockId)) },
    };
}

/** How far above the end of its transcript a pane must be scrolled for the check to mean anything. */
export const MIN_SCROLL_UP_PX = 200;

/** Whether the pane was scrolled up for real: a wheel turn a nested scroller took, or too short a transcript, leaves it at the bottom, where "the same position" is the default. */
export function scrollArmed(before, minPx = MIN_SCROLL_UP_PX) {
    return Boolean(before) && before.range - before.scrollTop >= minPx;
}

/** Whether a scroll position came back: the same row at the top, about where it was. */
export function scrollRestored(before, after, tolerancePx = 40) {
    if (!before || !after) return false;
    if (before.node == null || before.node !== after.node) return false;
    return Math.abs((before.offset ?? 0) - (after.offset ?? 0)) <= tolerancePx;
}

/**
 * After a recycle (the driver already reconnected): reads the draft and the
 * scroll position back. A pass needs both.
 */
export async function checkState(driver, armed) {
    const got = await driver.eval(`${COMPOSER(armed.draft.blockId)}?.value ?? null`);
    const after = await driver.eval(SCROLL_PROBE(armed.scroll.blockId));
    const draftOk = got === armed.draft.text;
    // A scroll that wasn't armed tests nothing: at the bottom, "restored" is what happens anyway.
    const armedOk = armed.scroll.scrolled && scrollArmed(armed.scroll.before);
    const scrollOk = armedOk && scrollRestored(armed.scroll.before, after);
    return {
        pass: draftOk && scrollOk,
        draft: { pass: draftOk, expected: armed.draft.text, got },
        scroll: {
            pass: scrollOk,
            ...(armedOk ? {} : { notArmed: "the transcript could not be scrolled up before the recycle (too short, or the wheel turn didn't move it): nothing was tested" }),
            before: armed.scroll.before,
            after,
        },
    };
}

// ── the recycle itself ──────────────────────────────────────────────────────

/** What a sample says about the recycle: the app and renderer totals, and how many documents the main window's page holds. */
export function recycleMark(sample, atSeconds) {
    const g = groupSample(sample);
    const main = mainPage(sample.pages ?? []);
    const mb = (bytes) => (bytes == null ? null : Math.round((bytes / MB) * 10) / 10);
    return {
        atSeconds,
        appMB: mb(g.app.private),
        rendererMB: mb(g.groups.renderer.private),
        rendererProcs: g.groups.renderer.procs,
        // Blink's document counter for the page's renderer: every document in it (frames too), so it is
        // not 1; a recycle must not make it grow (a back/forward-cache entry would).
        documents: main?.dom?.documents ?? null,
        pages: (sample.pages ?? []).length,
        // What the main page holds, to tell a restored page from a live one at the same point.
        nodes: main?.dom?.nodes ?? null,
        jsHeapMB: main?.jsHeapUsed != null ? Math.round((main.jsHeapUsed / MB) * 10) / 10 : null,
        layoutObjects: main?.layoutObjects ?? null,
        listeners: main?.dom?.jsEventListeners ?? null,
        toolBlocks: main?.probe?.toolBlocks ?? null,
        toolTextKB: main?.probe?.toolTextKB ?? null,
    };
}

/** What the page says when a command refused: the flash notice's text, if any. */
const FLASH_TEXT = `[...document.querySelectorAll('.notification-bubble')].map((e) => e.innerText.replace(/\s+/g, ' ').trim()).join(' | ')`;

/**
 * Runs the recycle command in the main window and measures it: a sample just
 * before; the command, from the palette as a user would; then a sample at each
 * of `afterSeconds` after it, each with a forced GC; and the draft and scroll
 * position read back. `recycled` is false when the page was never replaced
 * (the command refused: `blocked` has the notice's words).
 * @param {{ driver: any, memtree: string, port: number, rootPid: number, label: string, command: string, afterSeconds?: number[] }} o
 */
export async function runRecycle({ driver, memtree, port, rootPid, label, command, afterSeconds = [5, 30, 60] }) {
    const armed = await armState(driver);
    const before = recycleMark(await takeSample({ port, memtree, rootPid, label: `${label}-before-recycle`, gc: true }), 0);
    // A reload keeps the page's CDP target (and its id), so the old document is told by a mark it carries.
    const mark = `lab-${Date.now().toString(36)}`;
    await driver.eval(`window.__labDocument = ${JSON.stringify(mark)}`);

    // "none" presses nothing: the control of a pair, with the same arming and the same samples.
    const control = command === "none";
    if (!control) await driver.paletteCommand(command);
    const t0 = Date.now();

    let replaced = control;
    for (let waited = 0; waited < 45_000 && !replaced; waited += 500) {
        await sleep(500);
        replaced = (await driver.eval("window.__labDocument ?? null").catch(() => null)) !== mark;
    }
    if (!replaced) {
        const blocked = await driver.eval(FLASH_TEXT).catch(() => "");
        return { command, recycled: false, blocked: blocked || "no notice", before };
    }
    if (!control) await driver.reconnect();

    const after = [];
    for (const s of afterSeconds) {
        const wait = t0 + s * 1000 - Date.now();
        if (wait > 0) await sleep(wait);
        after.push(recycleMark(await takeSample({ port, memtree, rootPid, label: `${label}-recycle-${s}s`, gc: true }), s));
    }
    const state = await checkState(driver, armed);
    return { command, recycled: !control, control, before, after, state };
}

/** The recycle's saving, in MB of app and renderer private working set, from the settled mark (the last one) against the mark before. */
export function recycleSaving(rec) {
    if (!rec?.recycled || rec.control) return null;
    const settled = rec.after.at(-1);
    const d = (a, b) => (a == null || b == null ? null : Math.round((a - b) * 10) / 10);
    return { appMB: d(rec.before.appMB, settled.appMB), rendererMB: d(rec.before.rendererMB, settled.rendererMB) };
}
