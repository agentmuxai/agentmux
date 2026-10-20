// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

import { Pill } from "./Pill";

afterEach(cleanup);

describe("Pill", () => {
    it("is neutral by default and carries its words", () => {
        render(() => <Pill>managed</Pill>);
        const pill = screen.getByText("managed");
        expect(pill.classList.contains("ui-pill")).toBe(true);
        expect(pill.classList.contains("ui-pill--neutral")).toBe(true);
    });

    it("takes its tone, a decorative icon and a tooltip", () => {
        const { container } = render(() => (
            <Pill tone="warn" icon="circle-up" title="AgentMux is validated against 2.1.0">
                2.1.0 available
            </Pill>
        ));
        const pill = screen.getByText("2.1.0 available");
        expect(pill.classList.contains("ui-pill--warn")).toBe(true);
        expect(pill.getAttribute("title")).toBe("AgentMux is validated against 2.1.0");
        const icon = container.querySelector("i");
        expect(icon?.className).toBe("fa-solid fa-circle-up");
        expect(icon?.getAttribute("aria-hidden")).toBe("true");
    });

    it("leaves out the icon when there is none and keeps a caller's own class", () => {
        const { container } = render(() => (
            <Pill tone="ok" class="mine">
                Running
            </Pill>
        ));
        expect(container.querySelector("i")).toBeNull();
        expect(screen.getByText("Running").classList.contains("mine")).toBe(true);
    });
});
