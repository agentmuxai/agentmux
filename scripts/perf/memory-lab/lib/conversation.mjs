// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A long, tool-heavy scripted conversation for the fake Claude
// (scripts/ui-screenshots/fake-claude): the workload of the memory lab
// (docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md §2).
//
// Each turn: thinking, a few texts, 8-20 tool calls (Bash with 40-300 lines of
// ANSI output, Read of 80-400 code lines, Grep with 20-80 hits, Edit with a
// patch, TodoWrite) and a 2-6 KB markdown reply with code blocks: roughly the
// shape of a real coding turn, 50-150 KB. A seed makes it repeatable, so two
// builds play the same conversation.
//
//   node scripts/perf/memory-lab/lib/conversation.mjs <out.json> [turns=400] [seed=1] [realistic]
//
// "realistic" uses the CLI's own pacing (35 ms per streamed chunk, tools that
// take 0.2-2.5 s), so a turn lasts tens of seconds; the default compresses a
// session into minutes.

import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The scripted conversation, as the object fake-claude reads. */
export function generateConversation({ turns: TURNS = 400, seed: seed0 = 1, realistic: REALISTIC = false } = {}) {
    let seed = seed0;
    const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
    const int = (a, b) => a + Math.floor(rnd() * (b - a + 1));
    const ms = (fast, lo, hi) => (REALISTIC ? int(lo, hi) : fast);
    const pick = (a) => a[int(0, a.length - 1)];
    const WORDS = "the a pane agent memory renderer process window terminal buffer scrollback heap layout store reducer event stream turn tool result markdown block frame virtual list node row height measure flush queue scheduler socket srv backend frontend cache limit budget pressure release retain allocate fragment compact collect".split(" ");
    const sentence = (n = int(8, 22)) => {
        const w = Array.from({ length: n }, () => pick(WORDS));
        w[0] = w[0][0].toUpperCase() + w[0].slice(1);
        return w.join(" ") + ".";
    };
    const para = (s = int(2, 6)) => Array.from({ length: s }, () => sentence()).join(" ");
    const ident = () => pick(WORDS) + pick(["Count", "Bytes", "Rows", "State", "Queue", "Store", "Limit", "Frame"]);
    const codeLine = (i) =>
        pick([
            `    const ${ident()} = ${ident()}.${pick(["get", "set", "map", "filter", "reduce"])}((x) => x.${ident()} + ${i});`,
            `    if (${ident()} > ${int(1, 9999)}) return ${ident()}(${ident()}, ${int(0, 99)});`,
            `export function ${ident()}(${ident()}: number, ${ident()}: string): ${pick(["void", "number", "string[]", "Promise<void>"])} {`,
            `    // ${sentence()}`,
            `    ${ident()}.push({ id: ${i}, ${ident()}: "${pick(WORDS)}", ${ident()}: ${int(0, 1e6)} });`,
            `}`,
            ``,
            `    let ${ident()} = await fetch${ident()}(${ident()}, { timeout: ${int(100, 5000)} });`,
        ]);
    const fileLines = (n) => Array.from({ length: n }, (_, i) => `${String(i + 1).padStart(6)}\t${codeLine(i)}`).join("\n");
    const ansiColor = () => pick(["\x1b[32m", "\x1b[33m", "\x1b[31m", "\x1b[36m", "\x1b[0m"]);
    const bashOut = (n) =>
        Array.from({ length: n }, (_, i) =>
            pick([
                `${ansiColor()}✓\x1b[0m ${pick(WORDS)}/${ident()}.test.ts > ${sentence(6)} (${int(1, 400)} ms)`,
                `   Compiling ${pick(WORDS)}-${pick(WORDS)} v0.${int(1, 40)}.${int(0, 9)}`,
                `${pick(["src", "crates", "frontend"])}/${pick(WORDS)}/${ident()}.${pick(["ts", "rs", "tsx"])}:${int(1, 900)}:${int(1, 80)}: ${sentence(7)}`,
                `[${i}] ${new Date(1760000000000 + i * 1000).toISOString()} INFO ${pick(WORDS)}: ${sentence(10)}`,
            ])
        ).join("\n");
    const grepOut = (n) =>
        Array.from({ length: n }, () => `${pick(["frontend", "crates/srv/src"])}/${pick(WORDS)}/${ident()}.${pick(["ts", "rs"])}:${int(1, 2000)}:${codeLine(int(0, 99)).trim()}`).join("\n");
    const mdReply = () => {
        const parts = [`## ${sentence(5)}`, para(), "", "- " + sentence(), "- " + sentence(), "- " + sentence(), "", para()];
        for (let k = 0; k < int(1, 3); k++) {
            parts.push("", "```ts", ...Array.from({ length: int(8, 40) }, (_, i) => codeLine(i)), "```", "", para(int(1, 3)));
        }
        parts.push("", "| a | b | c |", "|---|---|---|", ...Array.from({ length: int(2, 8) }, () => `| ${pick(WORDS)} | ${int(1, 999)} | ${sentence(4)} |`));
        return parts.join("\n");
    };

    const turns = [];
    for (let t = 0; t < TURNS; t++) {
        const steps = [{ thinking: para(int(3, 8)) }, { text: sentence() }];
        const nTools = int(8, 20);
        for (let k = 0; k < nTools; k++) {
            const kind = pick(["Bash", "Bash", "Read", "Read", "Grep", "Edit", "Glob", "TodoWrite"]);
            const path = `D:/demo/acme-web/src/${pick(WORDS)}/${ident()}.ts`;
            if (kind === "Bash") {
                const n = int(40, 300);
                steps.push({ tool: "Bash", input: { command: `npm test -- ${pick(WORDS)} 2>&1 | tail -${n}`, description: sentence(5) }, result: bashOut(n), runMs: ms(int(50, 300), 600, 2500) });
            } else if (kind === "Read") {
                const n = int(80, 400);
                steps.push({ tool: "Read", input: { file_path: path }, result: fileLines(n), extra: { type: "text", file: { filePath: path, numLines: n, startLine: 1, totalLines: n } }, runMs: ms(50, 150, 400) });
            } else if (kind === "Grep") {
                steps.push({ tool: "Grep", input: { pattern: ident(), path: "frontend", output_mode: "content" }, result: grepOut(int(20, 80)), runMs: ms(50, 200, 600) });
            } else if (kind === "Glob") {
                steps.push({ tool: "Glob", input: { pattern: `src/**/${pick(WORDS)}*.ts` }, result: Array.from({ length: int(5, 60) }, () => `D:/demo/acme-web/src/${pick(WORDS)}/${ident()}.ts`).join("\n"), runMs: 50 });
            } else if (kind === "Edit") {
                const oldS = Array.from({ length: int(3, 15) }, (_, i) => codeLine(i)).join("\n");
                const newS = Array.from({ length: int(3, 20) }, (_, i) => codeLine(i + 1)).join("\n");
                steps.push({ tool: "Edit", input: { file_path: path, old_string: oldS, new_string: newS }, result: `The file ${path} has been updated successfully.`, runMs: ms(50, 200, 600) });
            } else {
                steps.push({
                    tool: "TodoWrite",
                    input: { todos: Array.from({ length: int(3, 7) }, (_, i) => ({ content: sentence(5), status: pick(["pending", "in_progress", "completed"]), activeForm: sentence(4) })) },
                    result: "Todos have been modified successfully.",
                    runMs: ms(30, 100, 200),
                });
            }
            if (rnd() < 0.3) steps.push({ text: para(int(1, 2)) });
        }
        steps.push({ text: mdReply() });
        turns.push({ steps, usage: { input_tokens: int(2000, 9000), output_tokens: int(300, 3000), cache_creation_input_tokens: 0, cache_read_input_tokens: int(10000, 90000) }, costUsd: 0.05 });
    }
    const script = { model: "claude-sonnet-5-5", account: "dev@acme.example", msPerChunk: REALISTIC ? 35 : 4, turns };
    return script;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
    const [out, turns, seed, mode] = process.argv.slice(2);
    if (!out) {
        console.error("usage: conversation.mjs <out.json> [turns=400] [seed=1] [realistic]");
        process.exit(2);
    }
    const script = generateConversation({ turns: Number(turns ?? 400), seed: Number(seed ?? 1), realistic: mode === "realistic" });
    const json = JSON.stringify(script);
    writeFileSync(out, json);
    console.log(`${script.turns.length} turns, ${(json.length / 1048576).toFixed(1)} MB, ${(json.length / script.turns.length / 1024).toFixed(0)} KB/turn`);
}
