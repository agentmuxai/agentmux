// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Semantic colour in rendered markdown (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md
 * §2, P1): a closed set of `am-*` span classes survives the sanitizer, nothing
 * else does, a half-streamed tag never flashes as raw text, the stylesheet maps
 * each class to its theme colour, and the Operator Config entry that tells
 * agents about it names exactly the classes that render.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import { Markdown } from "./markdown";
import { AM_SPAN_CLASSES } from "./markdown-semantic";

afterEach(() => cleanup());

const mount = (text: string) => render(() => <Markdown text={text} scrollable={false} />).container;

describe("am-* span classes survive the sanitizer", () => {
    it.each(AM_SPAN_CLASSES)("keeps %s", (cls) => {
        const el = mount(`Status: <span class="${cls}">word</span>`).querySelector("span." + cls);
        expect(el?.textContent).toBe("word");
    });

    it("keeps a colour class combined with am-badge", () => {
        const el = mount(`<span class="am-badge am-error">P1</span> broken`).querySelector("span.am-badge");
        expect([...el!.classList]).toEqual(["am-badge", "am-error"]);
    });

    it("drops unknown classes, including unknown am-* ones, and keeps the text", () => {
        const c = mount(`<span class="am-ok am-bogus rainbow">fine</span>`);
        const el = c.querySelector("span.am-ok");
        expect([...el!.classList]).toEqual(["am-ok"]);
        expect(c.querySelector(".am-bogus, .rainbow")).toBeNull();
    });

    it("drops style and event-handler attributes", () => {
        const el = mount(`<span class="am-warn" style="color:red" onclick="alert(1)">x</span>`).querySelector("span.am-warn")!;
        expect(el.getAttribute("style")).toBeNull();
        expect(el.getAttribute("onclick")).toBeNull();
    });

    it("still keeps the syntax highlighter's own classes", () => {
        const c = mount("```js\nconst a = 'x';\n```");
        expect(c.querySelector("[class*='hljs-']")).not.toBeNull();
    });
});

describe("a half-streamed tag never shows as raw text", () => {
    function stream(first: string, second: string) {
        const [text, setText] = createSignal(first);
        const c = render(() => <Markdown text={text()} streaming={true} scrollable={false} />).container;
        return { c, finish: () => setText(second) };
    }

    it.each([
        ['All tests <span class="am-o', "an open tag cut inside the class"],
        ["All tests <spa", "an open tag cut inside the name"],
        ['All tests <span class="am-ok">pass</sp', "a closing tag cut short"],
    ])("%s (%s)", (partial) => {
        const { c, finish } = stream(partial, 'All tests <span class="am-ok">pass</span> now.');
        expect(c.textContent).not.toMatch(/<\/?s(p(an?)?)?/);
        finish();
        expect(c.querySelector("span.am-ok")?.textContent).toBe("pass");
    });

    it("shows the same text in full once streaming ends", () => {
        const [streaming, setStreaming] = createSignal(true);
        const c = render(() => <Markdown text="a <span" streaming={streaming()} scrollable={false} />).container;
        setStreaming(false);
        expect(c.textContent).toContain("a <span");
    });
});

describe("stylesheet: each class uses its theme colour", () => {
    const scss = readFileSync(join(__dirname, "markdown.scss"), "utf8");
    const expected: Record<(typeof AM_SPAN_CLASSES)[number], string | null> = {
        "am-ok": "--success-text-color",
        "am-warn": "--warning-text-color",
        "am-error": "--error-text-color",
        "am-info": "--info-color",
        "am-muted": "--secondary-text-color",
        "am-added": "--success-text-color",
        "am-removed": "--error-text-color",
        "am-badge": null,
    };

    it.each(Object.entries(expected))("%s", (cls, colourVar) => {
        const m = new RegExp(`\\.${cls}\\s*\\{([^}]*)\\}`).exec(scss);
        expect(m, `no .${cls} rule in markdown.scss`).not.toBeNull();
        if (colourVar) expect(m![1]).toContain(`var(${colourVar})`);
    });
});

describe("Operator Config tells agents exactly the classes that render", () => {
    const manifest = JSON.parse(
        readFileSync(join(__dirname, "../../../crates/srv/operator-config-seed.json"), "utf8")
    ) as { version: number; entries: { id: string; instructions: string }[] };
    const entry = manifest.entries.find((e) => e.id === "operator-config-rich-output");

    it("has a rich-output entry, in a manifest generation newer than the first", () => {
        expect(entry).toBeDefined();
        expect(manifest.version).toBeGreaterThanOrEqual(2);
    });

    it("names every allowed class", () => {
        for (const cls of AM_SPAN_CLASSES) expect(entry!.instructions).toContain(cls);
    });

    it("names no class the sanitizer would strip", () => {
        const named = new Set(entry!.instructions.match(/\bam-[a-z]+\b/g));
        for (const cls of named) expect(AM_SPAN_CLASSES).toContain(cls);
    });
});
