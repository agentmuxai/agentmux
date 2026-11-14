// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The kit's close control: one ✕ with an accessible name, sizes as variants, and a click that reaches the host. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CloseButton } from "./CloseButton";

afterEach(cleanup);

const button = (c: HTMLElement) => c.querySelector("button")!;

describe("CloseButton", () => {
    it("is a button named Close by default, with a ✕ icon that screen readers skip", () => {
        const { container } = render(() => <CloseButton />);
        const b = button(container);
        expect(b.getAttribute("type")).toBe("button");
        expect(b.getAttribute("aria-label")).toBe("Close");
        const icon = b.querySelector("i")!;
        expect(icon.classList.contains("fa-xmark")).toBe(true);
        expect(icon.getAttribute("aria-hidden")).toBe("true");
    });

    it("says what it does when asked, and runs the host's click", () => {
        const onClick = vi.fn();
        const { container } = render(() => <CloseButton label="Dismiss" onClick={onClick} />);
        fireEvent.click(button(container));
        expect(button(container).getAttribute("aria-label")).toBe("Dismiss");
        expect(onClick).toHaveBeenCalledOnce();
    });

    it("takes its size as a variant class, comfortable by default, and the host's class beside it", () => {
        const { container } = render(() => (
            <>
                <CloseButton />
                <CloseButton size="sm" class="host-rule" tooltip={false} />
                <CloseButton size="header" />
            </>
        ));
        const all = [...container.querySelectorAll("button")];
        expect(all[0].classList.contains("ui-close--md")).toBe(true);
        expect(all[1].classList.contains("ui-close--sm") && all[1].classList.contains("host-rule")).toBe(true);
        expect(all[2].classList.contains("ui-close--header")).toBe(true);
    });

    it("does not act while disabled", () => {
        const onClick = vi.fn();
        const { container } = render(() => <CloseButton disabled onClick={onClick} />);
        fireEvent.click(button(container));
        expect(onClick).not.toHaveBeenCalled();
    });
});
