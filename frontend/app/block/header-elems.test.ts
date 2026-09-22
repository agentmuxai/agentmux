// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

import { isInteractiveHeaderElem, partitionHeaderElems } from "./header-elems";

const text = (t: string, extra: Partial<HeaderText> = {}): HeaderElem =>
    ({ elemtype: "text", text: t, ...extra }) as HeaderElem;

describe("isInteractiveHeaderElem", () => {
    it("treats a textbutton with an onClick as interactive", () => {
        // termViewModel's "Multi Input ON" — the case that regressed:
        // a real control that was moved into a hover-only tooltip.
        expect(
            isInteractiveHeaderElem({
                elemtype: "textbutton",
                text: "Multi Input ON",
                title: "Input will be sent to all connected terminals (click to disable)",
                onClick: vi.fn(),
            } as HeaderElem)
        ).toBe(true);
    });

    it("treats a textbutton with no onClick as passive", () => {
        expect(isInteractiveHeaderElem({ elemtype: "textbutton", text: "label" } as HeaderElem)).toBe(false);
    });

    it("treats a noAction iconbutton as passive", () => {
        // termViewModel's exit-code indicator: an icon used purely as a
        // status glyph. Belongs in the tooltip with the rest of the status
        // text, not pinned to the row.
        expect(
            isInteractiveHeaderElem({
                elemtype: "iconbutton",
                icon: "xmark-large",
                title: "Exit Code: 1",
                noAction: true,
            } as HeaderElem)
        ).toBe(false);
    });

    it("treats a clickable iconbutton as interactive, and a disabled one as passive", () => {
        expect(isInteractiveHeaderElem({ elemtype: "iconbutton", icon: "x", click: vi.fn() } as HeaderElem)).toBe(true);
        expect(
            isInteractiveHeaderElem({ elemtype: "iconbutton", icon: "x", click: vi.fn(), disabled: true } as HeaderElem)
        ).toBe(false);
    });

    it("treats plain text as passive, but text with an onClick as interactive", () => {
        expect(isInteractiveHeaderElem(text("user@host:~"))).toBe(false);
        expect(isInteractiveHeaderElem(text("click me", { onClick: vi.fn() }))).toBe(true);
    });

    it("treats inherently-interactive element types as interactive", () => {
        expect(isInteractiveHeaderElem({ elemtype: "input", value: "" } as HeaderElem)).toBe(true);
        expect(
            isInteractiveHeaderElem({ elemtype: "toggleiconbutton", icon: "x", active: (() => true) as any } as HeaderElem)
        ).toBe(true);
        expect(isInteractiveHeaderElem({ elemtype: "connectionbutton", icon: "x", text: "" } as HeaderElem)).toBe(true);
        expect(isInteractiveHeaderElem({ elemtype: "menubutton", items: [] } as unknown as HeaderElem)).toBe(true);
    });

    it("treats a div as interactive when it has a handler of its own", () => {
        expect(isInteractiveHeaderElem({ elemtype: "div", children: [], onClick: vi.fn() } as HeaderElem)).toBe(true);
        expect(isInteractiveHeaderElem({ elemtype: "div", children: [] } as HeaderElem)).toBe(false);
    });

    it("treats a div as interactive when any descendant is, however deeply nested", () => {
        const nested = {
            elemtype: "div",
            children: [text("a"), { elemtype: "div", children: [text("b", { onClick: vi.fn() })] }],
        } as HeaderElem;
        expect(isInteractiveHeaderElem(nested)).toBe(true);

        const inert = {
            elemtype: "div",
            children: [text("a"), { elemtype: "div", children: [text("b")] }],
        } as HeaderElem;
        expect(isInteractiveHeaderElem(inert)).toBe(false);
    });
});

describe("partitionHeaderElems", () => {
    it("keeps interactive elements inline and sends passive ones to the tooltip", () => {
        const multiInput = { elemtype: "textbutton", text: "Multi Input ON", onClick: vi.fn() } as HeaderElem;
        const oscTitle = text("user@host:~/projects");
        const exitCode = { elemtype: "iconbutton", icon: "xmark-large", noAction: true } as HeaderElem;

        const { inline, tooltip } = partitionHeaderElems([oscTitle, multiInput, exitCode]);

        expect(inline).toEqual([multiInput]);
        expect(tooltip).toEqual([oscTitle, exitCode]);
    });

    it("preserves relative order within each partition", () => {
        const a = text("a");
        const b = { elemtype: "textbutton", text: "b", onClick: vi.fn() } as HeaderElem;
        const c = text("c");
        const d = { elemtype: "textbutton", text: "d", onClick: vi.fn() } as HeaderElem;

        const { inline, tooltip } = partitionHeaderElems([a, b, c, d]);

        expect(inline).toEqual([b, d]);
        expect(tooltip).toEqual([a, c]);
    });

    it("returns everything in the tooltip when nothing is interactive", () => {
        const elems = [text("a"), text("b")];
        expect(partitionHeaderElems(elems)).toEqual({ inline: [], tooltip: elems });
    });

    it("returns two empty lists for an empty input", () => {
        expect(partitionHeaderElems([])).toEqual({ inline: [], tooltip: [] });
    });

    it("does not descend into a div — a mixed div stays whole and inline", () => {
        // A div's children are laid out by the div itself; splitting them
        // across two render sites would break whatever grouping it exists
        // to express. One interactive descendant pins the whole div inline.
        const div = {
            elemtype: "div",
            children: [text("status"), { elemtype: "textbutton", text: "act", onClick: vi.fn() }],
        } as HeaderElem;

        const { inline, tooltip } = partitionHeaderElems([div]);

        expect(inline).toEqual([div]);
        expect(tooltip).toEqual([]);
    });
});
