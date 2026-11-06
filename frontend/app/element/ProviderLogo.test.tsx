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
