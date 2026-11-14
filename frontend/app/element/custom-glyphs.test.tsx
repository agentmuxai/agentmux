// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The app's own glyphs (custom-glyphs.scss): the kit's icons take them by `custom@name`, the stylesheet draws them as
 * a mask in the text colour, and Compact in the composer strip uses one where "compress" (a size glyph, which the
 * tool panel keeps for its own size control) said the wrong thing. Stylesheets and components are read as text, like
 * PaneTabStrip.progress-style.test.ts.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { Button } from "@/app/element/ui/Button";
import { UiIcon } from "@/app/element/ui/shared";

afterEach(cleanup);

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string): string => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");

describe("UiIcon", () => {
    it("is a Font Awesome solid icon by bare name", () => {
        const { container } = render(() => <UiIcon name="compress" />);
        const i = container.querySelector("i")!;
        expect(i.className).toBe("fa-solid fa-compress");
        expect(i.getAttribute("aria-hidden")).toBe("true");
    });

    it("is one of the app's own glyphs by custom@name, as makeIconClass names them", () => {
        const { container } = render(() => <UiIcon name="custom@arrows-to-dot" />);
        expect(container.querySelector("i")!.className).toBe("fa-kit fa-arrows-to-dot");
    });

    it("follows its name when the icon changes (a toggle's compress and expand)", () => {
        const [name, setName] = createSignal("expand");
        const { container } = render(() => <UiIcon name={name()} />);
        expect(container.querySelector("i")!.className).toBe("fa-solid fa-expand");
        setName("compress");
        expect(container.querySelector("i")!.className).toBe("fa-solid fa-compress");
        setName("custom@arrows-to-dot");
        expect(container.querySelector("i")!.className).toBe("fa-kit fa-arrows-to-dot");
    });

    it("reaches a Button's icon the same way", () => {
        const { container } = render(() => <Button icon="custom@arrows-to-dot" aria-label="Compact" />);
        expect(container.querySelector("button i")!.className).toBe("fa-kit fa-arrows-to-dot");
    });
});

describe("custom-glyphs.scss", () => {
    const css = read("custom-glyphs.scss");

    it("defines the arrows-to-dot glyph", () => {
        expect(css).toMatch(/@include glyph\(\s*arrows-to-dot,/);
    });

    it("draws a glyph as a mask over the text colour, so it follows the text and scales with the font", () => {
        const mixin = css.slice(css.indexOf("@mixin glyph"), css.indexOf("// Four arrows in to a dot"));
        expect(mixin).toMatch(/background-color: currentColor;/);
        expect(mixin).toMatch(/(?<!-webkit-)mask: url\("data:image\/svg\+xml,/);
        expect(mixin).toMatch(/-webkit-mask: url\("data:image\/svg\+xml,/);
        expect(mixin).toMatch(/width: 1em;\s+height: 1em;/);
        expect(mixin).toMatch(/content: "";/);
    });
});

describe("where the size glyph and the history glyph are used", () => {
    it("Compact in the composer strip is the arrows-to-dot glyph, not the size glyph", () => {
        const strip = read("../view/agent/components/AgentComposerStrip.tsx");
        expect(strip).toContain('icon={compact() ? "custom@arrows-to-dot" : undefined}');
        expect(strip).not.toMatch(/"compress"/);
    });

    it("the tool panel keeps compress for its size control, paired with expand", () => {
        const parts = read("../view/agent/components/tool-block-parts.tsx");
        expect(parts).toContain('icon={props.view.tall() ? "compress" : "expand"}');
    });

    it("the stylesheet is loaded by the app", () => {
        expect(read("../app.tsx")).toContain('import "./element/custom-glyphs.scss";');
    });
});
