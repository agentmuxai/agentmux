// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Callouts in rendered markdown (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §3,
 * P2): GitHub's alert syntax, `> [!WARNING]`, renders as a titled callout in
 * the kind's theme colour; anything that isn't exactly that syntax stays an
 * ordinary blockquote; and the Operator Config entry tells agents about it.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import { Markdown } from "./markdown";
import { ALERT_KINDS } from "./remark-github-alerts";

afterEach(() => cleanup());

const mount = (text: string) => render(() => <Markdown text={text} scrollable={false} />).container;

const TITLES: Record<(typeof ALERT_KINDS)[number], string> = {
    note: "Note",
    tip: "Tip",
    important: "Important",
    warning: "Warning",
    caution: "Caution",
};

describe("GitHub alert syntax renders as a callout", () => {
    it.each(ALERT_KINDS)("[!%s]", (kind) => {
        const c = mount(`> [!${kind.toUpperCase()}]\n> Closing this removes the repro.`);
        const box = c.querySelector(`div.markdown-alert.markdown-alert-${kind}`);
        expect(box, "callout container").not.toBeNull();
        expect(box!.querySelector(".markdown-alert-title")?.textContent).toBe(TITLES[kind]);
        expect(box!.textContent).toContain("Closing this removes the repro.");
        expect(c.textContent).not.toContain(`[!`);
        expect(c.querySelector("blockquote")).toBeNull();
    });

    it("accepts a lower-case marker, as GitHub does", () => {
        const c = mount("> [!note]\n> body");
        expect(c.querySelector("div.markdown-alert-note")).not.toBeNull();
    });

    it("keeps everything after the marker line, including more paragraphs and lists", () => {
        const c = mount("> [!TIP]\n> First.\n>\n> - one\n> - two");
        const box = c.querySelector("div.markdown-alert-tip")!;
        expect(box.textContent).toContain("First.");
        expect(box.querySelectorAll("li")).toHaveLength(2);
    });

    it("renders the title alone while the body hasn't streamed in yet", () => {
        const [text, setText] = createSignal("> [!WARNING]");
        const c = render(() => <Markdown text={text()} streaming={true} scrollable={false} />).container;
        expect(c.querySelector("div.markdown-alert-warning .markdown-alert-title")?.textContent).toBe("Warning");
        expect(c.textContent).not.toContain("[!");
        setText("> [!WARNING]\n> Careful.");
        expect(c.querySelector("div.markdown-alert-warning")?.textContent).toContain("Careful.");
    });
});

describe("anything else stays an ordinary blockquote", () => {
    it.each([
        ["> [!UNKNOWN]\n> body", "an unknown kind"],
        ["> This mentions [!NOTE] mid-sentence.", "a marker mid-sentence"],
        ["> [!NOTE] text on the same line", "text after the marker on its line"],
        ["> Plain quote.", "no marker at all"],
    ])("%s (%s)", (md) => {
        const c = mount(md);
        expect(c.querySelector("blockquote")).not.toBeNull();
        expect(c.querySelector(".markdown-alert")).toBeNull();
    });
});

describe("sanitizer", () => {
    it("keeps only the callout classes on a raw div", () => {
        const c = mount(`<div class="markdown-alert markdown-alert-bogus evil" style="color:red">x</div>`);
        const el = c.querySelector("div.markdown-alert")!;
        expect([...el.classList]).toEqual(["markdown-alert"]);
        expect(el.getAttribute("style")).toBeNull();
    });
});

describe("stylesheet: each kind uses its theme colour", () => {
    const scss = readFileSync(join(__dirname, "markdown.scss"), "utf8");
    const expected: Record<(typeof ALERT_KINDS)[number], string> = {
        note: "--info-color",
        tip: "--success-text-color",
        important: "--accent-color",
        warning: "--warning-text-color",
        caution: "--error-text-color",
    };

    it.each(Object.entries(expected))("%s", (kind, colourVar) => {
        const m = new RegExp(`\\.markdown-alert-${kind}\\s*\\{([^}]*)\\}`).exec(scss);
        expect(m, `no .markdown-alert-${kind} rule in markdown.scss`).not.toBeNull();
        expect(m![1]).toContain(`var(${colourVar})`);
    });
});

describe("Operator Config tells agents about callouts", () => {
    const manifest = JSON.parse(
        readFileSync(join(__dirname, "../../../crates/srv/operator-config-seed.json"), "utf8")
    ) as { version: number; entries: { id: string; instructions: string }[] };
    const entry = manifest.entries.find((e) => e.id === "operator-config-rich-output")!;

    it("is in a manifest generation newer than P1's", () => {
        expect(manifest.version).toBeGreaterThanOrEqual(3);
    });

    it("names every kind in GitHub's syntax", () => {
        for (const kind of ALERT_KINDS) expect(entry.instructions).toContain(`[!${kind.toUpperCase()}]`);
    });
});
