// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// A favicon that is missing or fails to load shows a generic globe instead of a blank slot.

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { FaviconImage } from "./FaviconImage";

afterEach(cleanup);

describe("FaviconImage", () => {
    it("shows the favicon when there is one", () => {
        const { container } = render(() => <FaviconImage src="https://a.test/favicon.ico" class="fav" />);
        const img = container.querySelector("img");
        expect(img?.getAttribute("src")).toBe("https://a.test/favicon.ico");
        expect(img?.className).toBe("fav");
        expect(container.querySelector(".favicon-globe")).toBeNull();
    });

    it("shows a globe when the page has no favicon", () => {
        const { container } = render(() => <FaviconImage src="" />);
        expect(container.querySelector("img")).toBeNull();
        expect(container.querySelector(".favicon-globe")?.className).toContain("fa-globe");
    });

    it("shows a globe when the favicon fails to load (a 404)", () => {
        const { container } = render(() => <FaviconImage src="https://a.test/missing.ico" />);
        fireEvent.error(container.querySelector("img")!);
        expect(container.querySelector("img")).toBeNull();
        expect(container.querySelector(".favicon-globe")).not.toBeNull();
    });

    it("gives the next favicon its own try after one failed", () => {
        const [src, setSrc] = createSignal("https://a.test/missing.ico");
        const { container } = render(() => <FaviconImage src={src()} />);
        fireEvent.error(container.querySelector("img")!);
        expect(container.querySelector(".favicon-globe")).not.toBeNull();
        setSrc("https://b.test/favicon.ico");
        expect(container.querySelector("img")?.getAttribute("src")).toBe("https://b.test/favicon.ico");
        expect(container.querySelector(".favicon-globe")).toBeNull();
    });
});
