// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// In-page half of one-way-soak.mjs. Evaluated in the AgentMux page over CDP
// AFTER bench-page.js (it reuses window.__fcb's pane discovery and the live
// file-subject module); defines window.__ows.
//
// Streams scripted, tool-heavy assistant turns into each pane's real output
// subscription as Claude stream-json, so the translator, parser, flush queue,
// stream scheduler and every tool renderer run exactly as for a live agent.
// The script is built to hit the cases that move a following transcript:
// tool previews that stream in and then complete to a SHORTER result (Bash,
// Write), a long result (Read), parallel tools, an Edit diff, code blocks that
// re-highlight, and the end of a turn. No agent and no tokens.
//
// docs/specs/SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md §6.

(() => {
    if (!window.__fcb) throw new Error("load bench-page.js first");
    if (window.__ows) window.__ows.stop?.();

    const F = window.__fcb;
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const ndjson64 = (o) => {
        const bytes = new TextEncoder().encode(JSON.stringify(o) + String.fromCharCode(10));
        let s = "";
        for (const b of bytes) s += String.fromCharCode(b);
        return btoa(s);
    };
    const lines = (n, f) => Array.from({ length: n }, (_, i) => f(i)).join("\n");

    const S = (window.__ows = { running: false, turns: 0, subjects: [] });

    // Deterministic per-run pseudo-random, so two runs stream the same thing.
    let seed = 1;
    const rnd = () => {
        seed = (seed * 1103515245 + 12345) & 0x7fffffff;
        return seed / 0x7fffffff;
    };
    const between = (a, b) => a + Math.floor(rnd() * (b - a + 1));

    const panes = new Map();
    function pane(blockId) {
        if (panes.has(blockId)) return panes.get(blockId);
        const P = makePane(blockId);
        panes.set(blockId, P);
        return P;
    }
    function makePane(blockId) {
        const subj = F.mps.getFileSubject(blockId, "output");
        S.subjects.push(subj);
        let k = 0;
        const id = (p) => `${p}_fcb-ows-${blockId.slice(0, 6)}-${S.turns}-${k++}`;
        const send = (o) => subj.next({ fileop: "append", data64: ndjson64(o) });
        const ev = (event) => send({ type: "stream_event", event });

        /** One assistant message: text streamed in chunks, then tool calls with streamed input. */
        async function message({ text = "", tools = [] }) {
            const msgId = id("msg");
            ev({
                type: "message_start",
                message: { id: msgId, type: "message", role: "assistant", content: [], model: "soak", usage: { input_tokens: 1, output_tokens: 1 } },
            });
            const content = [];
            let index = 0;
            if (text) {
                ev({ type: "content_block_start", index, content_block: { type: "text", text: "" } });
                const step = Math.max(8, Math.ceil(text.length / between(6, 14)));
                for (let i = 0; i < text.length; i += step) {
                    ev({ type: "content_block_delta", index, delta: { type: "text_delta", text: text.slice(i, i + step) } });
                    await sleep(between(30, 90));
                }
                ev({ type: "content_block_stop", index });
                content.push({ type: "text", text });
                index++;
            }
            for (const t of tools) {
                t.id = id("toolu");
                ev({ type: "content_block_start", index, content_block: { type: "tool_use", id: t.id, name: t.name, input: {} } });
                const json = JSON.stringify(t.input);
                const step = Math.max(16, Math.ceil(json.length / between(4, 10)));
                for (let i = 0; i < json.length; i += step) {
                    ev({ type: "content_block_delta", index, delta: { type: "input_json_delta", partial_json: json.slice(i, i + step) } });
                    await sleep(between(25, 70));
                }
                ev({ type: "content_block_stop", index });
                content.push({ type: "tool_use", id: t.id, name: t.name, input: t.input });
                index++;
            }
            send({ type: "assistant", message: { id: msgId, type: "message", role: "assistant", content, model: "soak" } });
            ev({ type: "message_delta", delta: { stop_reason: tools.length ? "tool_use" : "end_turn" }, usage: { output_tokens: 1 } });
            ev({ type: "message_stop" });
        }

        /** Results for tools already sent, one user event each (as the CLI does). */
        async function results(tools) {
            for (const t of tools) {
                await sleep(between(200, 1400)); // the tool runs
                send({
                    type: "user",
                    message: { role: "user", content: [{ type: "tool_result", tool_use_id: t.id, content: t.result, is_error: false }] },
                    tool_use_result: t.structured ?? { stdout: t.result, stderr: "", interrupted: false },
                });
            }
        }

        return { message, results, send };
    }

    /** One full scripted turn for one pane. */
    async function runTurn(blockId) {
        const P = pane(blockId);
        const n = S.turns;
        const bash = {
            name: "Bash",
            input: { command: lines(between(4, 12), (i) => `echo "stage ${i}" && sleep 0.1`), description: "Run the staged build" },
            result: lines(between(1, 3), (i) => `stage ${i} ok`),
        };
        await P.message({
            text: `Turn ${n}: checking the build first.\n\n` + lines(between(2, 5), (i) => `- step ${i + 1} of the plan`),
            tools: [bash],
        });
        await P.results([bash]);

        // A Write whose preview (the whole file) is far taller than its result.
        const write = {
            name: "Write",
            input: { file_path: `/tmp/ows/module_${n}.ts`, content: lines(between(20, 45), (i) => `export const value${i} = ${i} * ${n};`) },
            result: `File created successfully at: /tmp/ows/module_${n}.ts`,
            structured: { type: "create", filePath: `/tmp/ows/module_${n}.ts` },
        };
        await P.message({ text: "Writing the new module.", tools: [write] });
        await P.results([write]);

        // Two Reads in parallel, results long.
        const reads = [0, 1].map((j) => ({
            name: "Read",
            input: { file_path: `/tmp/ows/input_${n}_${j}.ts` },
            result: lines(between(15, 40), (i) => `${String(i + 1).padStart(5)}\tconst line${i} = "${"x".repeat(between(10, 60))}";`),
        }));
        await P.message({ text: "Reading the two inputs.", tools: reads });
        await P.results(reads);

        // An Edit.
        const edit = {
            name: "Edit",
            input: {
                file_path: `/tmp/ows/module_${n}.ts`,
                old_string: lines(between(2, 6), (i) => `export const value${i} = ${i} * ${n};`),
                new_string: lines(between(3, 8), (i) => `export const value${i} = ${i} * ${n} + 1;`),
            },
            result: `The file /tmp/ows/module_${n}.ts has been updated.`,
        };
        await P.message({ text: "Applying the fix.", tools: [edit] });
        await P.results([edit]);

        // Closing text with a code block (re-highlights after streaming).
        const code = "```ts\n" + lines(between(4, 14), (i) => `export function step${n}_${i}(x: number): number { return x * ${i + 1}; }`) + "\n```";
        await P.message({ text: `Done with turn ${n}. The new helpers:\n\n${code}\n\nAll stages passed.` });
        P.send({ type: "result", subtype: "success", is_error: false, duration_ms: 1000, num_turns: 1, result: "ok", session_id: "fcb-ows" });
    }

    /** Stream turns into every selected pane until `untilMs` (performance.now() time) or stop(). */
    S.run = async ({ minutes, gapMs = 1500, seed: s = 1 }) => {
        if (!F.mps) throw new Error("could not load the stream subject module");
        seed = s;
        S.running = true;
        const until = performance.now() + minutes * 60_000;
        const ids = [...F.panes.keys()];
        while (S.running && performance.now() < until) {
            await Promise.all(ids.map((b) => runTurn(b)));
            S.turns++;
            await sleep(gapMs);
        }
        S.running = false;
        return { turns: S.turns, panes: ids.length };
    };

    S.stop = () => {
        S.running = false;
        for (const subj of S.subjects) {
            try {
                subj.release();
            } catch {}
        }
        S.subjects = [];
        panes.clear();
    };
})();
