// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
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

    it("shows a question below its card's row, before the details, and gives focus back to the card when it goes", async () => {
        const spy = widthOf(406); // 2 columns
        const [asked, setAsked] = createSignal<string | null>("a");
        const { container } = render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                selectedKey="a"
                askKey={asked()}
                renderCard={(i) => <Card title={i} aria-label={i} />}
                renderDetail={(i) => <div>details of {i}</div>}
                renderAsk={(i) => <button>Delete {i}?</button>}
            />
        ));
        const order = () =>
            [...container.querySelector(".ui-card-grid")!.children].map((el) =>
                el.classList.contains("ui-card-detail") ? "D" : el.classList.contains("ui-card-ask") ? "Q" : el.getAttribute("data-card-key"),
            );
        expect(order()).toEqual(["a", "b", "Q", "D", "c", "d", "e"]);
        setAsked("e");
        expect(order()).toEqual(["a", "b", "D", "c", "d", "e", "Q"]);
        expect(screen.getByRole("button", { name: "Delete e?" })).toBeTruthy();
        // Closed while focus is on the question's button, which leaves with it: the card the question was about has it.
        screen.getByRole("button", { name: "Delete e?" }).focus();
        setAsked(null);
        expect(order()).toEqual(["a", "b", "D", "c", "d", "e"]);
        await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: "e" })));
        spy.mockRestore();
    });

    it("stops at maxColumns however wide it is, and sets each track to at most maxCardWidth", () => {
        const spy = widthOf(2400);
        const { container } = render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                minCardWidth={270}
                maxColumns={4}
                maxCardWidth={380}
                renderCard={(i) => <Card title={i} aria-label={i} />}
            />
        ));
        const grid = container.querySelector(".ui-card-grid") as HTMLElement;
        expect(grid.classList.contains("ui-card-grid--capped")).toBe(true);
        expect(grid.style.gridTemplateColumns).toBe("repeat(4, minmax(0, 380px))");
        spy.mockRestore();
    });

    it("drops columns as the grid narrows, by the card's minimum, at the widths the Connectors spec lists", () => {
        // 270 px minimum, 6 px gap, 10 px padding each side: 2 columns from 566, 3 from 842, 4 from 1118 (SPEC_CONNECTORS_ONE_LIST §4.7).
        const columns = (w: number) => Math.min(columnsFor(w - 20, 270, 6), 4);
        expect([280, 339, 340, 565, 566, 841, 842, 1117, 1118, 2400].map(columns)).toEqual([1, 1, 1, 1, 2, 2, 3, 3, 4, 4]);
    });

    it("keeps the order of the cards when the column count changes, moving only the details", () => {
        const [w, setW] = createSignal(1118);
        const spy = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
            return this.classList.contains("ui-card-grid") ? w() : 0;
        });
        let notify: () => void = () => {};
        const original = globalThis.ResizeObserver;
        globalThis.ResizeObserver = class {
            constructor(cb: () => void) {
                notify = cb;
            }
            observe() {}
            disconnect() {}
            unobserve() {}
        } as unknown as typeof ResizeObserver;
        const { container } = render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                selectedKey="b"
                minCardWidth={270}
                maxColumns={4}
                renderCard={(i) => <Card title={i} aria-label={i} />}
                renderDetail={(i) => <div>details of {i}</div>}
            />
        ));
        const order = () =>
            [...container.querySelector(".ui-card-grid")!.children].map((el) =>
                el.classList.contains("ui-card-detail") ? "D" : el.getAttribute("data-card-key"),
            );
        // 1118 px: four columns, so b's row is a b c d.
        expect(order()).toEqual(["a", "b", "c", "d", "D", "e"]);
        setW(566);
        notify();
        expect(order()).toEqual(["a", "b", "D", "c", "d", "e"]);
        setW(300);
        notify();
        expect(order()).toEqual(["a", "b", "D", "c", "d", "e"]);
        expect(container.querySelectorAll(".ui-card-cell").length).toBe(5);
        globalThis.ResizeObserver = original;
        spy.mockRestore();
    });

    it("names the details as a region, labelled by the open card, and moves focus into them when asked", async () => {
        const spy = widthOf(406);
        const [sel, setSel] = createSignal<string | null>(null);
        render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                selectedKey={sel()}
                detailId="the-details"
                detailLabel={(i) => `${i} details`}
                focusDetails
                renderCard={(i) => <Card title={i} aria-label={i} aria-controls="the-details" />}
                renderDetail={(i) => <div>details of {i}</div>}
            />
        ));
        setSel("b");
        const region = await screen.findByRole("region", { name: "b details" });
        expect(region.id).toBe("the-details");
        await waitFor(() => expect(document.activeElement).toBe(region));
        expect(screen.getByRole("button", { name: "b" }).getAttribute("aria-controls")).toBe("the-details");
        spy.mockRestore();
    });

    it("gives the details no role when no label is asked for", () => {
        const spy = widthOf(406);
        const { container } = render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                selectedKey="a"
                renderCard={(i) => <Card title={i} aria-label={i} />}
                renderDetail={() => <div>details</div>}
            />
        ));
        expect(container.querySelector(".ui-card-detail")!.getAttribute("role")).toBeNull();
        spy.mockRestore();
    });

    it("keeps the open details when the list refreshes with new objects", () => {
        const spy = widthOf(406);
        const [list, setList] = createSignal([{ id: "a", n: 1 }, { id: "b", n: 1 }]);
        render(() => (
            <CardGrid
                items={list()}
                keyOf={(i) => i.id}
                selectedKey="a"
                renderCard={(i) => <Card title={`${i.id}:${i.n}`} aria-label={i.id} />}
                renderDetail={() => <input aria-label="nickname" />}
            />
        ));
        const input = screen.getByLabelText("nickname");
        setList([{ id: "a", n: 2 }, { id: "b", n: 2 }]);
        expect(screen.getByLabelText("nickname")).toBe(input);
        expect(screen.getByText("a:2")).toBeTruthy();
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

    it("stays open when onClose refuses (busy), and keeps focus in the details", () => {
        const spy = widthOf(406);
        const [sel] = createSignal<string | null>("b");
        render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                selectedKey={sel()}
                onClose={() => false}
                renderCard={(i) => <Card title={i} aria-label={i} />}
                renderDetail={() => <input aria-label="nickname" />}
            />
        ));
        const input = screen.getByLabelText("nickname");
        input.focus();
        fireEvent.keyDown(input, { key: "Escape" });
        expect(screen.getByLabelText("nickname")).toBe(input);
        expect(document.activeElement).toBe(input);
        spy.mockRestore();
    });

    it("gives focus back to a card that isn't a Card (a view's own focusable card)", () => {
        const spy = widthOf(406);
        const [sel, setSel] = createSignal<string | null>("b");
        render(() => (
            <CardGrid
                items={items}
                keyOf={(i) => i}
                selectedKey={sel()}
                onClose={() => setSel(null)}
                renderCard={(i) => (
                    <div role="button" tabIndex={0} aria-label={`own ${i}`}>
                        {i}
                    </div>
                )}
                renderDetail={() => <input aria-label="nickname" />}
            />
        ));
        fireEvent.keyDown(screen.getByLabelText("nickname"), { key: "Escape" });
        expect(sel()).toBeNull();
        expect(document.activeElement).toBe(screen.getByRole("button", { name: "own b" }));
        spy.mockRestore();
    });

    it("finds a card whose key holds characters CSS.escape rewrites", () => {
        const spy = widthOf(406);
        const key = "agent\u0000github";
        const [sel, setSel] = createSignal<string | null>(key);
        render(() => (
            <CardGrid
                items={["x", key]}
                keyOf={(i) => i}
                selectedKey={sel()}
                onClose={() => setSel(null)}
                renderCard={(i) => <Card title={i} aria-label={i === key ? "observed" : i} />}
                renderDetail={() => <input aria-label="nickname" />}
            />
        ));
        fireEvent.keyDown(screen.getByLabelText("nickname"), { key: "Escape" });
        expect(document.activeElement).toBe(screen.getByRole("button", { name: "observed" }));
        spy.mockRestore();
    });
});
