// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Card } from "./Card";
import { CardGrid, columnsFor, rowEnd } from "./CardGrid";
import { StatusDot } from "./StatusDot";

afterEach(cleanup);

describe("StatusDot", () => {
    it("shows its tone and says its label", () => {
        render(() => <StatusDot tone="warn" label="Expired" />);
        const dot = screen.getByRole("img", { name: "Expired" });
        expect(dot.classList.contains("ui-status-dot--warn")).toBe(true);
        expect(dot.getAttribute("title")).toBe("Expired");
    });
});

describe("Card", () => {
    it("is one button that opens and closes from the keyboard", () => {
        const onSelect = vi.fn();
        render(() => <Card title="me@example.com" subtitle="Anthropic" selected onSelect={onSelect} aria-label="Account" />);
        const card = screen.getByRole("button", { name: "Account" });
        expect(card.getAttribute("aria-expanded")).toBe("true");
        fireEvent.keyDown(card, { key: "Enter" });
        fireEvent.keyDown(card, { key: " " });
        fireEvent.keyDown(card, { key: "Escape" });
        expect(onSelect).toHaveBeenCalledTimes(3);
    });

    it("doesn't close on Escape when it isn't open", () => {
        const onSelect = vi.fn();
        render(() => <Card title="x" onSelect={onSelect} aria-label="Card" />);
        fireEvent.keyDown(screen.getByRole("button", { name: "Card" }), { key: "Escape" });
        expect(onSelect).not.toHaveBeenCalled();
    });

    it("keeps its actions' clicks and keys to themselves", () => {
        const onSelect = vi.fn();
        const onOpen = vi.fn();
        const act = vi.fn();
        render(() => (
            <Card
                title="db1"
                onSelect={onSelect}
                onOpen={onOpen}
                aria-label="Remote"
                actions={
                    <button type="button" onClick={act}>
                        New terminal
                    </button>
                }
            />
        ));
        const action = screen.getByText("New terminal");
        fireEvent.click(action);
        fireEvent.dblClick(action);
        fireEvent.keyDown(action, { key: "Enter" });
        expect(act).toHaveBeenCalledTimes(1);
        expect(onSelect).not.toHaveBeenCalled();
        expect(onOpen).not.toHaveBeenCalled();
    });

    it("opens on double-click and passes right-click through", () => {
        const onOpen = vi.fn();
        const onContextMenu = vi.fn();
        render(() => <Card title="db1" onOpen={onOpen} onContextMenu={onContextMenu} aria-label="Remote" />);
        const card = screen.getByRole("button", { name: "Remote" });
        fireEvent.dblClick(card);
        fireEvent.contextMenu(card);
        expect(onOpen).toHaveBeenCalledTimes(1);
        expect(onContextMenu).toHaveBeenCalledTimes(1);
    });

    it("shows status, meta and a colour edge", () => {
        render(() => (
            <Card
                title="db1"
                aria-label="Remote"
                status={{ tone: "ok", label: "Connected" }}
                meta={[<span>Helper 0.59.9</span>, <span>2 sessions</span>]}
                edgeColor="#3b82f6"
                data-remote="db1"
            />
        ));
        const card = screen.getByRole("button", { name: "Remote" });
        expect(card.getAttribute("data-remote")).toBe("db1");
        expect(card.classList.contains("has-edge")).toBe(true);
        expect(screen.getByRole("img", { name: "Connected" })).toBeTruthy();
        expect(card.querySelectorAll(".ui-card-meta > *")).toHaveLength(2);
    });

    it("as a chip, has no meta line or expanded state", () => {
        render(() => <Card size="chip" title="GitHub" meta={[<span>x</span>]} aria-label="GitHub" />);
        const chip = screen.getByRole("button", { name: "GitHub" });
        expect(chip.getAttribute("aria-expanded")).toBeNull();
        expect(chip.querySelector(".ui-card-meta")).toBeNull();
    });
});

describe("CardGrid layout", () => {
    it("counts the columns an auto-fill grid fits", () => {
        expect(columnsFor(0, 200, 6)).toBe(1);
        expect(columnsFor(406, 200, 6)).toBe(2);
        expect(columnsFor(405, 200, 6)).toBe(1);
        expect(columnsFor(1000, 200, 6)).toBe(4);
    });

    it("puts the details after the last card of the open card's row", () => {
        // 3 columns, 7 cards: rows [0 1 2] [3 4 5] [6]
        expect(rowEnd(0, 3, 7)).toBe(2);
        expect(rowEnd(1, 3, 7)).toBe(2);
        expect(rowEnd(2, 3, 7)).toBe(2);
        expect(rowEnd(4, 3, 7)).toBe(5);
        expect(rowEnd(6, 3, 7)).toBe(6);
        expect(rowEnd(5, 1, 7)).toBe(5);
    });
});

describe("CardGrid", () => {
    const items = ["a", "b", "c", "d", "e"];
    const widthOf = (w: number) =>
        vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
            return this.classList.contains("ui-card-grid") ? w : 0;
        });

    it("shows the open card's details below its row", () => {
        const spy = widthOf(406); // 2 columns
        const [sel, setSel] = createSignal<string | null>("a");
        const { container } = render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                selectedKey={sel()}
                renderCard={(i) => <Card title={i} aria-label={i} />}
                renderDetail={(i) => <div data-testid="detail">details of {i}</div>}
            />
        ));
        const order = () => [...container.querySelector(".ui-card-grid")!.children].map((el) =>
            el.classList.contains("ui-card-detail") ? "D" : el.getAttribute("data-card-key"),
        );
        expect(order()).toEqual(["a", "b", "D", "c", "d", "e"]);
        expect(screen.getByTestId("detail").textContent).toBe("details of a");
        setSel("e");
        expect(order()).toEqual(["a", "b", "c", "d", "e", "D"]);
        setSel(null);
        expect(order()).toEqual(["a", "b", "c", "d", "e"]);
        spy.mockRestore();
    });

    it("closes on Escape in the details and gives the card focus back", () => {
        const spy = widthOf(406);
        const [sel, setSel] = createSignal<string | null>("b");
        render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                selectedKey={sel()}
                onClose={() => setSel(null)}
                renderCard={(i) => <Card title={i} aria-label={i} selected={sel() === i} />}
                renderDetail={() => <input aria-label="nickname" />}
            />
        ));
        fireEvent.keyDown(screen.getByLabelText("nickname"), { key: "Escape" });
        expect(sel()).toBeNull();
        expect(document.activeElement).toBe(screen.getByRole("button", { name: "b" }));
        spy.mockRestore();
    });
});
