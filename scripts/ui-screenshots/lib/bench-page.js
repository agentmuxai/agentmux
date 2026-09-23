// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// In-page half of full-conversation-bench.mjs. Evaluated in the AgentMux page
// over CDP; defines window.__fcb. Plain browser JavaScript (no imports at load
// time) so it can be sent as one Runtime.evaluate expression.
//
// Synthetic content goes straight into each pane's document store through the
// live module instance (`getPaneModel(blockId).dispatchDoc`), so the bench
// needs no agents and no tokens and is exactly repeatable. Every node it
// creates has an id starting with "fcb-", which is how a pane is recognised as
// holding only bench content.
//
// docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md, Phase 0.

(() => {
    if (window.__fcb) return "already installed";

    const SYN = "fcb-";
    const q = (arr, p) => {
        if (!arr.length) return null;
        const s = [...arr].sort((a, b) => a - b);
        return +s[Math.min(s.length - 1, Math.floor(p * s.length))].toFixed(1);
    };
    const sum = (arr) => +arr.reduce((a, b) => a + b, 0).toFixed(1);
    const nextFrames = (n = 2) =>
        new Promise((r) => {
            const step = (k) => (k === 0 ? r() : requestAnimationFrame(() => step(k - 1)));
            step(n);
        });

    // ~1 KB of markdown exercising headings, prose, code, lists and a table.
    const unit = (i) =>
        "## Section " +
        i +
        " — " +
        Math.random().toString(36).slice(2, 8) +
        "\n\n" +
        "Some **bold** prose with `inline code` and enough words to wrap a line or two at a normal pane width, repeated for realism. ".repeat(
            3
        ) +
        "\n\n```ts\nfunction f" +
        i +
        "(x: number): number {\n  return x * " +
        i +
        ";\n}\n```\n\n" +
        "- level one\n  - level two\n    - level three\n\n" +
        "| a | b | c | d |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |\n| 5 | 6 | 7 | 8 |\n\n";
    const markdown = (kb) => {
        let s = "";
        for (let i = 0; s.length < kb * 1024; i++) s += unit(i);
        return s;
    };
    const stdout = (lines) => Array.from({ length: lines }, (_, i) => `line ${i}: ${"x".repeat(60)}`).join("\n");

    const F = (window.__fcb = {
        panes: new Map(), // blockId -> { model }
        seq: 0,
        visibility: { hiddenDuring: false },
    });

    document.addEventListener("visibilitychange", () => {
        if (document.visibilityState !== "visible") F.visibility.hiddenDuring = true;
    });

    /**
     * Resolve a live module instance (the one the app is using). Prefer the URL
     * the page actually loaded (it carries the ?t= HMR stamp if the module was
     * hot-updated — that instance is the live one); the resource buffer holds
     * only ~250 entries, so fall back to the plain URL, which IS the live
     * instance for a module never hot-updated.
     */
    async function liveModule(path) {
        const re = new RegExp(path.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&") + "(\\?|$)");
        const urls = performance
            .getEntriesByType("resource")
            .map((e) => e.name)
            .filter((n) => re.test(n));
        return import(urls[urls.length - 1] || location.origin + path);
    }
    const registration = () => liveModule("/frontend/app/store/agent-pane-registration.ts");
    const documentStore = () => liveModule("/frontend/app/store/agent-document-store.ts");

    /** Every visible agent pane: block frames holding an agent composer. */
    F.discover = async () => {
        const reg = await registration();
        const docs = await documentStore();
        const out = [];
        for (const frame of document.querySelectorAll("[data-blockid]")) {
            const blockId = frame.getAttribute("data-blockid");
            const composer = frame.querySelector("textarea.agent-input");
            if (!composer || !frame.checkVisibility?.()) continue;
            const model = reg.getPaneModel(blockId);
            if (!model) continue;
            const nodes = docs.snapshot(blockId)?.nodes ?? null;
            out.push({
                blockId,
                model,
                nodeCount: nodes ? nodes.length : null,
                syntheticOnly: nodes ? nodes.every((n) => String(n.id).startsWith(SYN)) : null,
            });
        }
        return out;
    };

    F.init = async (blockIds) => {
        const found = await F.discover();
        const want = blockIds && blockIds.length ? found.filter((p) => blockIds.includes(p.blockId)) : found;
        F.panes = new Map(want.map((p) => [p.blockId, { model: p.model }]));
        return {
            visibility: document.visibilityState,
            panes: want.map(({ blockId, nodeCount, syntheticOnly }) => ({ blockId, nodeCount, syntheticOnly })),
            available: found.map((p) => p.blockId),
        };
    };

    F.clear = (blockIds = [...F.panes.keys()]) => {
        for (const id of blockIds) F.panes.get(id)?.model.dispatchDoc({ type: "UserClear" });
        return true;
    };

    /** Append `turns` synthetic turns to every selected pane. */
    F.inject = (turns, turnKb) => {
        for (const [, { model }] of F.panes) {
            const nodes = [];
            const now = Date.now();
            for (let t = 0; t < turns; t++) {
                const k = F.seq++;
                const ts = now - (turns - t) * 60_000;
                nodes.push({
                    type: "user_message",
                    id: `${SYN}user-${k}`,
                    message: `Synthetic request ${k}: explain the next section.`,
                    timestamp: ts,
                });
                nodes.push({
                    type: "tool",
                    id: `${SYN}tool-${k}`,
                    tool: "Bash",
                    toolName: "Bash",
                    params: { command: `echo synthetic ${k}` },
                    status: "success",
                    duration: 0.4,
                    result: { stdout: stdout(40), stderr: "", exitCode: 0 },
                    collapsed: true,
                    summary: `Bash echo synthetic ${k} (0.4s) ✓`,
                    timestamp: ts + 1000,
                });
                nodes.push({ type: "markdown", id: `${SYN}md-${k}`, content: markdown(turnKb), timestamp: ts + 2000 });
            }
            model.dispatchDoc({ type: "StreamFlush", newNodes: nodes, updatedNodes: [] });
        }
        return true;
    };

    F.domCounts = () => {
        const perPane = {};
        for (const id of F.panes.keys()) {
            const frame = document.querySelector(`[data-blockid="${CSS.escape(id)}"]`);
            perPane[id] = frame ? frame.getElementsByTagName("*").length : null;
        }
        return { total: document.getElementsByTagName("*").length, perPane };
    };

    // ── Measurement window ───────────────────────────────────────────────────

    F.startWindow = ({ streamKb, secs }) => {
        if (document.visibilityState !== "visible")
            throw new Error(`page is ${document.visibilityState} at window start`);
        const W = (F.win = {
            t0: performance.now(),
            loafs: [],
            gaps: [],
            events: [],
            keydowns: 0,
            dispatchMs: [],
            hiddenDuring: false,
        });
        F.visibility.hiddenDuring = false;
        W.po = new PerformanceObserver((l) => {
            for (const e of l.getEntries()) {
                W.loafs.push({
                    dur: e.duration,
                    blocking: e.blockingDuration,
                    forcedLayout: (e.scripts || []).reduce((a, s) => a + s.forcedStyleAndLayoutDuration, 0),
                    render: e.renderStart ? e.startTime + e.duration - e.renderStart : 0,
                });
            }
        });
        W.po.observe({ type: "long-animation-frame" });
        // Event Timing: key -> next paint. Entries under 16 ms are not reported
        // (spec minimum), so unreported keydowns are counted separately.
        W.eo = new PerformanceObserver((l) => {
            for (const e of l.getEntries()) if (e.name === "keydown") W.events.push(e.duration);
        });
        W.eo.observe({ type: "event", durationThreshold: 16 });
        W.kh = () => W.keydowns++;
        document.addEventListener("keydown", W.kh, true);
        let last = performance.now();
        W.raf = true;
        const f = (t) => {
            W.gaps.push(t - last);
            last = t;
            if (W.raf) requestAnimationFrame(f);
        };
        requestAnimationFrame(f);

        // Stream one synthetic message into every pane at ~11 commits/s.
        const commits = Math.max(1, Math.round((secs * 1000) / 90));
        W.done = Promise.all(
            [...F.panes].map(
                ([, { model }]) =>
                    new Promise((resolve) => {
                        const text = markdown(streamKb);
                        const id = `${SYN}stream-${F.seq++}`;
                        model.dispatchDoc({
                            type: "StreamFlush",
                            newNodes: [{ type: "markdown", id, content: "", timestamp: Date.now() }],
                            updatedNodes: [],
                        });
                        const step = Math.ceil(text.length / commits);
                        let k = 0;
                        const tick = () => {
                            k++;
                            const a = performance.now();
                            model.dispatchDoc({
                                type: "StreamFlush",
                                newNodes: [],
                                updatedNodes: [
                                    {
                                        type: "markdown",
                                        id,
                                        content: text.slice(0, Math.min(k * step, text.length)),
                                        timestamp: Date.now(),
                                    },
                                ],
                            });
                            W.dispatchMs.push(performance.now() - a);
                            if (k < commits) setTimeout(tick, 90);
                            else resolve();
                        };
                        setTimeout(tick, 90);
                    })
            )
        );
        return true;
    };

    F.finishWindow = async () => {
        const W = F.win;
        await W.done;
        await nextFrames(3);
        W.raf = false;
        for (const e of W.po.takeRecords())
            W.loafs.push({
                dur: e.duration,
                blocking: e.blockingDuration,
                forcedLayout: (e.scripts || []).reduce((a, s) => a + s.forcedStyleAndLayoutDuration, 0),
                render: e.renderStart ? e.startTime + e.duration - e.renderStart : 0,
            });
        for (const e of W.eo.takeRecords()) if (e.name === "keydown") W.events.push(e.duration);
        W.po.disconnect();
        W.eo.disconnect();
        document.removeEventListener("keydown", W.kh, true);
        const elapsed = performance.now() - W.t0;
        // Unreported keydowns were < 16 ms: count them at 8 ms for percentiles
        // and say so, rather than silently dropping them (which biases high).
        const unreported = Math.max(0, W.keydowns - W.events.length);
        const keyToPaint = [...W.events, ...Array(unreported).fill(8)];
        return {
            elapsedMs: +elapsed.toFixed(0),
            hiddenDuring: F.visibility.hiddenDuring,
            visibilityAtRead: document.visibilityState,
            frames: W.gaps.length,
            fps: +((W.gaps.length * 1000) / elapsed).toFixed(1),
            rafGapMs: {
                p50: q(W.gaps, 0.5),
                p95: q(W.gaps, 0.95),
                max: q(W.gaps, 1),
                over50: W.gaps.filter((g) => g > 50).length,
            },
            longFrames: {
                count: W.loafs.length,
                totalMs: sum(W.loafs.map((l) => l.dur)),
                shareOfTime: +(sum(W.loafs.map((l) => l.dur)) / elapsed).toFixed(3),
                blockingMs: sum(W.loafs.map((l) => l.blocking)),
                forcedLayoutMs: sum(W.loafs.map((l) => l.forcedLayout)),
                renderMs: sum(W.loafs.map((l) => l.render)),
                maxMs: W.loafs.length ? +Math.max(...W.loafs.map((l) => l.dur)).toFixed(1) : 0,
            },
            keyToPaintMs: W.keydowns
                ? {
                      keydowns: W.keydowns,
                      under16: unreported,
                      p50: q(keyToPaint, 0.5),
                      p95: q(keyToPaint, 0.95),
                      max: q(keyToPaint, 1),
                  }
                : null,
            dispatchMs: {
                commits: W.dispatchMs.length,
                p50: q(W.dispatchMs, 0.5),
                p95: q(W.dispatchMs, 0.95),
                max: q(W.dispatchMs, 1),
                total: sum(W.dispatchMs),
            },
        };
    };

    // ── Typing target: snapshot, guard, restore ──────────────────────────────
    //
    // Lessons from PR #3569: capture the draft (value, selection, owning block);
    // swallow any synthetic key whose target is not the captured composer so
    // it cannot type into anything else; restore through the block's LIVE
    // composer if this one was remounted (AgentFooter persists every input per
    // block, so the remounted footer shows the synthetic text), and if none is
    // mounted within 30 s, hand the draft back to the caller to report.

    F.armTyping = (blockId) => {
        const frame = document.querySelector(`[data-blockid="${CSS.escape(blockId)}"]`);
        const el = frame?.querySelector("textarea.agent-input");
        if (!el) throw new Error(`no composer in block ${blockId}`);
        el.focus();
        if (document.activeElement !== el) throw new Error(`could not focus the composer of block ${blockId}`);
        const T = (F.typing = {
            el,
            blockId,
            value: el.value,
            start: el.selectionStart,
            end: el.selectionEnd,
            stray: 0,
        });
        T.guard = (e) => {
            if (e.target !== T.el) {
                e.preventDefault();
                e.stopImmediatePropagation();
                T.stray++;
            }
        };
        document.addEventListener("keydown", T.guard, true);
        return true;
    };

    F.typingOk = () =>
        !!F.typing && F.typing.el.isConnected && document.activeElement === F.typing.el && F.typing.stray === 0;

    F.restoreTyping = async () => {
        const T = F.typing;
        if (!T) return { ok: true, how: "nothing to restore" };
        document.removeEventListener("keydown", T.guard, true);
        const write = (el) => {
            el.value = T.value;
            try {
                el.setSelectionRange(T.start, T.end);
            } catch {}
            el.dispatchEvent(new Event("input", { bubbles: true }));
        };
        const done = (how) => {
            F.typing = null;
            return { ok: true, how, stray: T.stray };
        };
        if (T.el.isConnected) {
            write(T.el);
            return done("captured composer");
        }
        const find = () => document.querySelector(`[data-blockid="${CSS.escape(T.blockId)}"] textarea.agent-input`);
        for (let i = 0; i < 300; i++) {
            const el = find();
            if (el) {
                write(el);
                return done("remounted composer");
            }
            await new Promise((r) => setTimeout(r, 100));
        }
        return { ok: false, blockId: T.blockId, draft: T.value, stray: T.stray };
    };

    return "installed";
})();
