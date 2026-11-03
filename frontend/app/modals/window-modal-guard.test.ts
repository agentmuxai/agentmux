// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The guard of docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md §5: a dialog about one pane or one agent
 * covers only that pane, so the window-level ways to open one only go down. It counts, per source file, the places
 * that draw a modal over the whole window:
 *
 *  - `window-scope`:   `scope="window"` (a `<Modal>`, `<ConfirmModal>` or `<ClaudeLoginPanel>` that says so);
 *  - `default-scope`:  a `<Modal>` or `<ConfirmModal>` with no `scope` at all (the default is the window);
 *  - `open-modal`:     a call to the imperative `openModal(` / `modalsModel.openModal(`, which renders at the window root;
 *  - `native-dialog`:  `window.confirm`, `window.alert` or `window.prompt`, which freeze the whole renderer.
 *
 * A file may not have more of any kind than `window-modal-sites.json` says, and the file lists nothing a file no longer
 * has. A new dialog about a pane uses `openPaneModal(blockId, ...)` or `<Modal scope="pane">`; one that really is
 * about the whole app or window (the spec's keep table) is added to the baseline with the reason in the PR.
 * Lowering a count: run this test with `UPDATE_WINDOW_MODAL_BASELINE=1` and commit the file.
 */

import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = join(__dirname, "..");
const BASELINE = join(__dirname, "window-modal-sites.json");

type Kind = "window-scope" | "default-scope" | "open-modal" | "native-dialog";
type Counts = Partial<Record<Kind, number>>;

/** Source files of the frontend, not tests. */
function sources(dir: string, out: string[] = []): string[] {
    for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) {
            if (name !== "node_modules") sources(p, out);
        } else if (/\.(ts|tsx)$/.test(name) && !/\.(test|spec)\.(ts|tsx)$/.test(name) && !name.endsWith(".d.ts")) {
            out.push(p);
        }
    }
    return out;
}

/** The text of a JSX opening tag that starts at `from` (just after the tag name), up to its closing `>`. */
function openingTag(src: string, from: number): string {
    let depth = 0;
    let quote = "";
    for (let i = from; i < src.length; i++) {
        const c = src[i];
        if (quote) {
            if (c === quote && src[i - 1] !== "\\") quote = "";
        } else if (c === '"' || c === "'" || c === "`") {
            quote = c;
        } else if (c === "{") depth++;
        else if (c === "}") depth--;
        else if (c === ">" && depth === 0 && src[i - 1] !== "=") return src.slice(from, i);
    }
    return src.slice(from);
}

function countsOf(src: string): Counts {
    // Comments carry examples of all of these: drop them before counting.
    const code = src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
    const c: Counts = {};
    const add = (k: Kind, n: number) => n > 0 && (c[k] = (c[k] ?? 0) + n);
    add("window-scope", (code.match(/\bscope=(?:"window"|\{"window"\})/g) ?? []).length);
    let defaults = 0;
    for (const m of code.matchAll(/<(?:Modal|ConfirmModal)\b/g)) {
        if (!/\bscope=/.test(openingTag(code, m.index! + m[0].length))) defaults++;
    }
    add("default-scope", defaults);
    add("open-modal", (code.match(/(?<![.\w])(?:modalsModel\.)?openModal\(/g) ?? []).length - (code.match(/\bfunction openModal\(|\bopenModal\([^)]*\)\s*\{/g) ?? []).length);
    add("native-dialog", (code.match(/\bwindow\.(?:confirm|alert|prompt)\(/g) ?? []).length);
    return c;
}

function current(): Record<string, Counts> {
    const out: Record<string, Counts> = {};
    for (const file of sources(ROOT).sort()) {
        const counts = countsOf(readFileSync(file, "utf8"));
        if (Object.keys(counts).length) out[relative(ROOT, file).split("\\").join("/")] = counts;
    }
    return out;
}

describe("window-level modals only go down", () => {
    const now = current();
    if (process.env.UPDATE_WINDOW_MODAL_BASELINE) {
        writeFileSync(BASELINE, JSON.stringify(now, null, 2) + "\n");
    }
    const baseline = JSON.parse(readFileSync(BASELINE, "utf8")) as Record<string, Counts>;

    it("no file has a window-level dialog the baseline does not list", () => {
        const grew: string[] = [];
        for (const [file, counts] of Object.entries(now)) {
            for (const [kind, n] of Object.entries(counts) as [Kind, number][]) {
                const allowed = baseline[file]?.[kind] ?? 0;
                if (n > allowed) grew.push(`${file}: ${kind} ${n} (allowed ${allowed})`);
            }
        }
        expect(
            grew,
            "A dialog about one pane or agent must cover only that pane: use openPaneModal(blockId, Component, props) or " +
                '<Modal scope="pane">. Only app-wide ones stay (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md §2, §4).'
        ).toEqual([]);
    });

    it("the baseline lists nothing that is gone (run with UPDATE_WINDOW_MODAL_BASELINE=1 and commit it)", () => {
        const stale: string[] = [];
        for (const [file, counts] of Object.entries(baseline)) {
            for (const [kind, n] of Object.entries(counts) as [Kind, number][]) {
                const have = now[file]?.[kind] ?? 0;
                if (have < n) stale.push(`${file}: ${kind} ${have} (baseline ${n})`);
            }
        }
        expect(stale).toEqual([]);
    });
});
