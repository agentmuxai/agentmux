// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DeclIconButton } from "./DeclIconButton";

afterEach(cleanup);

const button = (c: HTMLElement) => c.querySelector("button")!;

describe("DeclIconButton", () => {
    it("is a chrome button named by the decl's title, with its icon, class and colour", () => {
        const decl = { elemtype: "iconbutton", icon: "copy", title: "Copy", className: "from-decl", iconColor: "red" } as IconButtonDecl;
        const { container } = render(() => <DeclIconButton decl={decl} className="from-site" />);
        const b = button(container);
        expect(b.classList.contains("ui-button--chrome")).toBe(true);
        expect(b.classList.contains("ui-button")).toBe(false);
        expect(b.classList.contains("from-site") && b.classList.contains("from-decl")).toBe(true);
        expect(b.getAttribute("title")).toBe("Copy");
        expect(b.getAttribute("aria-label")).toBe("Copy");
        expect(b.style.color).toBe("red");
        expect(b.querySelector("i")).not.toBeNull();
        expect(b.hasAttribute("aria-pressed")).toBe(false);
    });

    it("inherits its colour unless the decl gives one, and shows a JSX icon as given", () => {
        const decl = { elemtype: "iconbutton", icon: <svg data-testid="glyph" /> } as IconButtonDecl;
        const { container } = render(() => <DeclIconButton decl={decl} />);
        expect(button(container).style.color).toBe("inherit");
        expect(container.querySelector("svg")).not.toBeNull();
        expect(container.querySelector("i")).toBeNull();
    });

    it("runs the click, and does nothing when disabled", () => {
        const click = vi.fn();
        const { container, unmount } = render(() => <DeclIconButton decl={{ elemtype: "iconbutton", icon: "copy", click } as IconButtonDecl} />);
        fireEvent.click(button(container));
        expect(click).toHaveBeenCalledOnce();
        unmount();
        const click2 = vi.fn();
        const r = render(() => <DeclIconButton decl={{ elemtype: "iconbutton", icon: "copy", click: click2, disabled: true } as IconButtonDecl} />);
        expect(button(r.container).disabled).toBe(true);
        fireEvent.click(button(r.container));
        expect(click2).not.toHaveBeenCalled();
    });

    it("is a toggle when the decl is: aria-pressed follows the signal, a click flips it, and the title says Active", () => {
        const [on, setOn] = createSignal(false);
        const active = Object.assign(on, { _set: setOn }) as unknown as ToggleIconButtonDecl["active"];
        const decl = { elemtype: "toggleiconbutton", icon: "thumbtack", title: "Pin", active } as ToggleIconButtonDecl;
        const { container } = render(() => <DeclIconButton decl={decl} />);
        const b = button(container);
        expect(b.getAttribute("aria-pressed")).toBe("false");
        expect(b.classList.contains("ui-button--toggle")).toBe(true);
        expect(b.getAttribute("title")).toBe("Pin");
        fireEvent.click(b);
        expect(on()).toBe(true);
        expect(b.getAttribute("aria-pressed")).toBe("true");
        expect(b.getAttribute("title")).toBe("Pin (Active)");
    });
});
