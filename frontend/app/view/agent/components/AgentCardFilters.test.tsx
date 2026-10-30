// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { matchesFilters, type CardFilter, type FilterableCard } from "../defaults/provider-access-model";
import { AgentCardFilters } from "./AgentCardFilters";

afterEach(() => cleanup());

const NOW = () => new Date("2026-10-29T12:00:00Z");
// antigravity is free, goose is local, and a provider the data doesn't know has neither tag.
const CARDS: FilterableCard[] = [
    { provider: "antigravity", installed: true },
    { provider: "unlisted", installed: true },
    { provider: "goose", installed: false },
];

function Harness(props: { installedKnown?: boolean; cards?: FilterableCard[] }) {
    const [selected, setSelected] = createSignal<CardFilter[]>([]);
    const cards = () => props.cards ?? CARDS;
    const shown = () => cards().filter((c) => matchesFilters(c, new Set(selected()), NOW())).length;
    return (
        <AgentCardFilters
            cards={cards()}
            selected={selected()}
            onChange={setSelected}
            shown={shown()}
            installedKnown={props.installedKnown ?? true}
            now={NOW}
        />
    );
}

const chip = (name: string) => screen.getByRole("button", { name: new RegExp(name) });

describe("AgentCardFilters", () => {
    it("shows each chip with the number of cards it would leave", () => {
        render(() => <Harness />);
        expect(chip("Free").textContent).toContain("1");
        expect(chip("Local").textContent).toContain("1");
        expect(chip("Installed").textContent).toContain("2");
    });

    it("leaves the Installed count out until every install probe has answered", () => {
        const { container } = render(() => <Harness installedKnown={false} />);
        expect(chip("Installed").textContent).not.toMatch(/\d/);
        expect(container.querySelectorAll(".ui-filter-chip-count").length).toBe(2);
    });

    it("says how many cards are shown only while a filter is on, and the chip follows the click", () => {
        render(() => <Harness />);
        expect(screen.getByTestId("agent-card-filters-status").textContent).toBe("");
        fireEvent.click(chip("Installed"));
        expect(chip("Installed").getAttribute("aria-pressed")).toBe("true");
        expect(screen.getByTestId("agent-card-filters-status").textContent).toBe("2 of 3 shown");
    });

    it("counts what selecting a chip would leave on top of the chips already on", () => {
        render(() => <Harness />);
        fireEvent.click(chip("Installed"));
        // Of the two installed cards, one is free and none is local.
        expect(chip("Free").textContent).toContain("1");
        expect(chip("Local").textContent).toContain("0");
    });

    it("says what is missing and offers to clear when nothing passes", () => {
        render(() => <Harness />);
        fireEvent.click(chip("Local"));
        fireEvent.click(chip("Installed"));
        expect(screen.getByTestId("agent-card-filters-empty").textContent).toContain("No agent is local and installed.");
        fireEvent.click(screen.getByTestId("agent-card-filters-clear"));
        expect(screen.queryByTestId("agent-card-filters-empty")).toBeNull();
        expect(chip("Local").getAttribute("aria-pressed")).toBe("false");
    });
});
