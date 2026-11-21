// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { SeatTag } from "./seat-tag";

afterEach(cleanup);

describe("SeatTag", () => {
    it("shows the role and qualifier in the role's colour, with a hidden lead word for a screen reader", () => {
        const { container } = render(() => <SeatTag seat={{ role: "leader", qualifier: "product" }} title="Leader · product — reports to the operator" />);
        const tag = container.querySelector<HTMLElement>(".ui-tag")!;
        expect(tag.classList.contains("ui-tag--seat-leader")).toBe(true);
        expect(tag.classList.contains("ui-tag--neutral")).toBe(false);
        expect(tag.textContent).toBe("Seat Leader · product");
        expect(tag.querySelector(".ui-tag-sr")!.textContent).toBe("Seat ");
        expect(tag.getAttribute("title")).toBe("Leader · product — reports to the operator");
    });

    it("takes one colour per role", () => {
        const roles = ["leader", "worker", "specialist", "platform_tester", "analyst", "super"] as const;
        const classes = roles.map((role) => {
            const { container, unmount } = render(() => <SeatTag seat={{ role, qualifier: null }} />);
            const c = [...container.querySelector(".ui-tag")!.classList].find((n) => n.startsWith("ui-tag--seat-"))!;
            unmount();
            return c;
        });
        expect(new Set(classes).size).toBe(6);
    });

    it("shows a role this build does not know as written, neutral", () => {
        render(() => <SeatTag seat={{ role: "overseer" as never, qualifier: null }} />);
        const tag = screen.getByText("overseer").closest(".ui-tag")!;
        expect(tag.classList.contains("ui-tag--neutral")).toBe(true);
        expect([...tag.classList].some((c) => c.startsWith("ui-tag--seat-"))).toBe(false);
    });
});
