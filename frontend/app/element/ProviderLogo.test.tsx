// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { ProviderLogo } from "./ProviderLogo";

afterEach(() => cleanup());

describe("ProviderLogo for Mux Code", () => {
    it("draws the four blocks cropped to their own box, so they fill the 1em box like the other logos", () => {
        const { container } = render(() => <ProviderLogo provider="muxcode" />);
        const svg = container.querySelector("svg");
        expect(svg?.getAttribute("viewBox")).toBe("64 64 384 384");
        expect(svg?.getAttribute("width")).toBe("1em");
        expect(svg?.querySelectorAll("rect")).toHaveLength(4);
    });
});

describe("ProviderLogo for a route's model vendors", () => {
    for (const vendor of ["openrouter", "xai", "ollama", "meta"]) {
        it(`draws ${vendor}'s own mark, not a letter`, () => {
            const { container } = render(() => <ProviderLogo provider={vendor} />);
            expect(container.querySelector("svg")).toBeTruthy();
            expect(container.querySelector("text")).toBeNull();
        });
    }

    it("still draws a letter for a vendor with no mark to use", () => {
        const { container } = render(() => <ProviderLogo provider="unmarked-vendor" />);
        expect(container.querySelector("text")?.textContent).toBe("U");
    });
});
