// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// In-page half of follow-soak.mjs. Evaluated in the AgentMux page over CDP; defines window.__fsk.
// It only READS the page (the scroller's follow state and geometry) and finds where to send real
// input; the input itself goes in from outside through CDP, as a person's would.
//
// docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md §7.3.
(() => {
    const K = (window.__fsk = {});

    /** The transcript scroller of a pane: the `.agent-document` that carries `data-follow-state`. */
    const scroller = (blockId) => {
        for (const frame of document.querySelectorAll(`[data-blockid="${blockId}"]`)) {
            const el = frame.querySelector(".agent-document[data-follow-state]");
            if (el) return el;
        }
        return null;
    };
    const visible = (el) => !!el && el.checkVisibility?.() !== false && el.getClientRects().length > 0;
    const center = (r) => ({ x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) });

    /** One reading of a pane: state and geometry, as the follow rule sees them. */
    K.sample = (blockId) => {
        const el = scroller(blockId);
        if (!el) return { blockId, found: false };
        const r = el.getBoundingClientRect();
        return {
            blockId,
            found: true,
            visible: visible(el),
            state: el.getAttribute("data-follow-state"),
            gap: el.scrollHeight - el.clientHeight - el.scrollTop,
            scrollTop: el.scrollTop,
            scrollHeight: el.scrollHeight,
            clientHeight: el.clientHeight,
            covered: el.hasAttribute("data-show-cover"),
            center: center(r),
        };
    };

    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    /** The follower eases toward the bottom (about 140 ms to cover 90 % of a step, follower.ts), so a
     *  following pane is legitimately a little above the bottom while content streams. */
    const QUIET_SETTLE_MS = 700;
    const FAR_FRACTION = 0.75;

    /**
     * Whether a pane that says it is following has stopped converging on the bottom. Two ways to be
     * stuck: it sits above the bottom through a stretch of at least QUIET_SETTLE_MS in which the
     * content did not change height (the follower would have arrived), or it stays more than
     * FAR_FRACTION of the viewport away for three readings in a row (the follower snaps a step that
     * large, so it should never persist). Returns at once when the pane is at the bottom, detached,
     * or hidden.
     */
    K.stuck = async (blockId, epsilon) => {
        let base = K.sample(blockId);
        let quietSince = performance.now();
        let far = 0;
        const deadline = performance.now() + 4000;
        while (performance.now() < deadline) {
            await sleep(100);
            const s = K.sample(blockId);
            if (!s.found || !s.visible || s.covered || s.state !== "following" || s.gap <= epsilon) return { stuck: false, sample: s };
            if (s.scrollHeight !== base.scrollHeight) {
                base = s;
                quietSince = performance.now();
                far = 0;
            } else if (performance.now() - quietSince >= QUIET_SETTLE_MS) {
                return { stuck: true, reason: `${s.gap.toFixed(1)} px above the bottom with the content unchanged for ${QUIET_SETTLE_MS} ms`, sample: s };
            }
            far = s.gap > FAR_FRACTION * s.clientHeight ? far + 1 : 0;
            if (far >= 3) return { stuck: true, reason: `${s.gap.toFixed(0)} px away (more than ${FAR_FRACTION * 100}% of the viewport) for three readings`, sample: s };
        }
        return { stuck: false, sample: K.sample(blockId) };
    };

    /** Wheel events the page received, by pane, to tell "the gesture never arrived" from "it was ignored". */
    const wheels = [];
    K.watchWheels = () => {
        if (K._watching) return;
        K._watching = true;
        document.addEventListener(
            "wheel",
            (e) => {
                const frame = e.target?.closest?.("[data-blockid]");
                const t = e.target instanceof Element ? e.target : null;
                // Plain transcript text, as opposed to a tool's own scroll box: only a wheel over the
                // former says anything about the pane's follow state. Content moves under the cursor
                // between aiming and the wheel arriving, so this is judged when the wheel lands.
                const plain = !!t && !!t.closest(".agent-markdown-block") && !t.closest(".scroll-handoff-box, .agent-tool-panel, pre, code");
                wheels.push({ at: performance.now(), blockId: frame?.getAttribute("data-blockid") ?? null, deltaY: e.deltaY, plain });
                if (wheels.length > 400) wheels.shift();
            },
            true
        );
    };
    /** Whether the latest wheel the pane received landed on plain transcript text. */
    K.lastWheelPlain = (blockId) => {
        const mine = wheels.filter((w) => w.blockId === blockId);
        return mine.length ? mine[mine.length - 1].plain : null;
    };
    K.wheelsSince = (blockId, sinceAgoMs) => wheels.filter((w) => w.blockId === blockId && performance.now() - w.at <= sinceAgoMs).length;
    K.elementAt = (x, y) => {
        const el = document.elementFromPoint(x, y);
        const frame = el?.closest?.("[data-blockid]");
        return { blockId: frame?.getAttribute("data-blockid") ?? null, cls: (el?.className ?? "").toString().slice(0, 50), tag: el?.tagName ?? null };
    };

    /** A point inside the transcript to send input to, or null. kind: "text" | "expander" | "scroller". */
    K.point = (blockId, kind, pick = 0) => {
        const el = scroller(blockId);
        if (!el || !visible(el)) return null;
        const box = el.getBoundingClientRect();
        if (kind === "scroller") return center(box);
        // The part of an element that is inside the transcript (clear of its scrollbar), or null.
        const clip = (r) => {
            const left = Math.max(r.left, box.left + 4);
            const right = Math.min(r.right, box.right - 24);
            const top = Math.max(r.top, box.top + 4);
            const bottom = Math.min(r.bottom, box.bottom - 4);
            return right - left >= 20 && bottom - top >= 8 ? { left, right, top, bottom, width: right - left, height: bottom - top } : null;
        };
        // A tool row's header toggles its panel; a markdown paragraph is plain text.
        const sel = kind === "expander" ? ".agent-tool-summary" : ".paragraph, p, li";
        const rects = [...el.querySelectorAll(sel)]
            .filter(visible)
            .map((n) => clip(n.getBoundingClientRect()))
            .filter(Boolean);
        if (rects.length === 0) return null;
        return center(rects[pick % rects.length]);
    };

    /**
     * A point on the transcript where a wheel-up scrolls the transcript itself: not over a nested
     * scroll box (a tool preview, a code block), which takes the wheel first and rightly leaves the
     * pane following. Null when there is none in view.
     */
    K.wheelPoint = (blockId, pick = 0) => {
        const root = scroller(blockId);
        if (!root || !visible(root)) return null;
        const box = root.getBoundingClientRect();
        const candidates = [];
        for (let i = 1; i <= 9; i++) {
            for (const fx of [0.3, 0.5, 0.7]) candidates.push({ x: Math.round(box.left + box.width * fx), y: Math.round(box.top + (box.height * i) / 10) });
        }
        const clean = candidates.filter((c) => {
            for (let el = document.elementFromPoint(c.x, c.y); el && el !== root; el = el.parentElement) {
                const o = getComputedStyle(el);
                const scrolls = /auto|scroll/.test(o.overflowY) && el.scrollHeight > el.clientHeight + 1;
                if (scrolls) return false;
            }
            const hit = document.elementFromPoint(c.x, c.y);
            if (!hit || !root.contains(hit)) return false;
            // Only plain text of the transcript: a hand-off box (a tool's log or preview) takes the
            // wheel for itself, so a wheel there says nothing about the pane's follow state.
            return !!hit.closest(".agent-markdown-block") && !hit.closest(".scroll-handoff-box, .agent-tool-panel, pre, code");
        });
        return clean.length ? clean[pick % clean.length] : null;
    };

    /**
     * How far the transcript was scrolled UP over the next `ms`, and the state it ended in. A wheel
     * over a nested scroll box (a tool preview) is taken by that box and moves the transcript not at
     * all; this tells that apart from a wheel that scrolled the transcript. Only a decrease of
     * scrollTop between two frames at the same content height counts: content that grows below (a
     * tall tool preview) raises the gap without scrolling anything, and content that shrinks makes
     * the browser clamp scrollTop down without any input.
     */
    K.watch = (blockId, ms) =>
        new Promise((resolve) => {
            let upScrolled = 0;
            let maxGap = 0;
            let prev = K.sample(blockId);
            const t0 = performance.now();
            const end = t0 + ms;
            // Every scroll event and wheel event of the window, to read a failure back afterwards.
            const trace = [];
            const el = scroller(blockId);
            const onScroll = () => {
                if (trace.length < 60) trace.push(`${Math.round(performance.now() - t0)}ms scroll top=${Math.round(el.scrollTop)} h=${el.scrollHeight}`);
            };
            const onWheel = (e) => {
                if (trace.length < 60) trace.push(`${Math.round(performance.now() - t0)}ms wheel dy=${e.deltaY} top=${Math.round(el.scrollTop)} h=${el.scrollHeight}`);
            };
            el?.addEventListener("scroll", onScroll, { passive: true });
            document.addEventListener("wheel", onWheel, { capture: true, passive: true });
            const tick = () => {
                const s = K.sample(blockId);
                if (s.found && prev.found) {
                    maxGap = Math.max(maxGap, s.gap);
                    if (s.scrollHeight === prev.scrollHeight && s.scrollTop < prev.scrollTop) upScrolled += prev.scrollTop - s.scrollTop;
                }
                prev = s;
                if (performance.now() < end) requestAnimationFrame(tick);
                else {
                    el?.removeEventListener("scroll", onScroll);
                    document.removeEventListener("wheel", onWheel, { capture: true });
                    resolve({ upScrolled, maxGap, state: s.state, gap: s.gap, trace });
                }
            };
            requestAnimationFrame(tick);
        });

    /**
     * Tool rows, straight into the pane's document: the stream-json streamer (one-way-soak-page.js)
     * does not produce tool nodes on this build, and tools are what move a following transcript
     * (a preview that is tall while it runs and short once it is done). Each round adds a running
     * Write whose preview is the whole file, then, a moment later, completes it to a one-line result.
     */
    let toolSeq = 0;
    let toolsRunning = false;
    K.stopTools = () => {
        toolsRunning = false;
    };
    K.runTools = async ({ minutes, gapMs = 2200 }) => {
        const F = window.__fcb;
        if (!F?.panes?.size) throw new Error("load bench-page.js and init it first");
        toolsRunning = true;
        const until = performance.now() + minutes * 60_000;
        const lines = (n, f) => Array.from({ length: n }, (_, i) => f(i)).join("\n");
        let turns = 0;
        while (toolsRunning && performance.now() < until) {
            await Promise.all(
                [...F.panes].map(async ([blockId, { model }], idx) => {
                    await new Promise((r) => setTimeout(r, idx * 400));
                    const k = toolSeq++;
                    const id = `fsk-tool-${blockId.slice(0, 6)}-${k}`;
                    const now = Date.now();
                    const base = { type: "tool", id, tool: "Write", toolName: "Write", timestamp: now };
                    model.dispatchDoc({
                        type: "StreamFlush",
                        newNodes: [{ ...base, params: { file_path: `/tmp/fsk/m${k}.ts`, content: lines(25 + (k % 20), (i) => `export const v${i} = ${i} * ${k};`) }, status: "running", collapsed: false }],
                        updatedNodes: [],
                    });
                    await new Promise((r) => setTimeout(r, 900 + (k % 5) * 200));
                    model.dispatchDoc({
                        type: "StreamFlush",
                        newNodes: [],
                        updatedNodes: [{ ...base, params: { file_path: `/tmp/fsk/m${k}.ts`, content: "x" }, status: "success", duration: 0.9, result: { stdout: `File created: /tmp/fsk/m${k}.ts`, stderr: "", exitCode: 0 }, collapsed: true, summary: `Write m${k}.ts ✓` }],
                    });
                })
            );
            turns++;
            await new Promise((r) => setTimeout(r, gapMs));
        }
        toolsRunning = false;
        return { turns };
    };

    /** The pane's composer, to move focus in and out of it. */
    K.focusComposer = (blockId, on) => {
        for (const frame of document.querySelectorAll(`[data-blockid="${blockId}"]`)) {
            const t = frame.querySelector("textarea.agent-input");
            if (t) {
                if (on) t.focus();
                else t.blur();
                return true;
            }
        }
        return false;
    };

    K.viewport = () => ({ width: window.innerWidth, height: window.innerHeight });
})();
