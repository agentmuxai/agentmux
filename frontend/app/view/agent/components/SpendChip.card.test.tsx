// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The spend chip on a Swarm card, with the real popover: the Spend cap panel is portaled to the body, and Solid's
 * delegated events still bubble from a portal to its logical parent, the card. A press in the panel must not reach the
 * card's handlers (they select the agent's pane, and the card's mousedown keeps a field from taking focus).
 */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@floating-ui/dom", () => ({ autoUpdate: vi.fn(() => vi.fn()) }));
vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined,
    usePaneOverlay: vi.fn(),
}));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
}));
vi.mock("./AgentSpendPanel", () => ({
    AgentSpendPanel: () => (
        <div data-testid="chip-panel">
            <input data-testid="chip-field" />
            <button data-testid="chip-set">Set cap</button>
        </div>
    ),
}));

import { SPEND_TODAY_META_KEY } from "../spend-stopped";
import { SpendChip } from "./SpendChip";

afterEach(cleanup);

const now = () => 1_700_000_000_000;
const meta = { [SPEND_TODAY_META_KEY]: { day: Math.floor(now() / 86_400_000), spent: 4.2, cap: 50 } };

function mountOnCard() {
    const seen = { pointerdown: vi.fn(), mousedown: vi.fn(), click: vi.fn(), keydown: vi.fn(), contextmenu: vi.fn() };
    const result = render(() => (
        <div
            data-testid="card"
            onPointerDown={seen.pointerdown}
            onMouseDown={(e) => {
                // The real card cancels a mousedown that is not on one of its own controls.
                seen.mousedown();
                e.preventDefault();
            }}
            onClick={seen.click}
            onKeyDown={seen.keydown}
            onContextMenu={seen.contextmenu}
        >
            <SpendChip meta={meta} now={now} blockId="blk-1" agentName="AgentY" />
        </div>
    ));
    fireEvent.click(result.getByTestId("spend-chip-button"));
    for (const fn of Object.values(seen)) fn.mockClear();
    return { ...result, seen };
}

describe("SpendChip on a card, with the real popover", () => {
    it("opens the panel in the body, outside the card's own DOM", () => {
        const { container } = mountOnCard();
        const panel = screen.getByTestId("chip-panel");
        expect(container.contains(panel)).toBe(false);
        expect(document.body.contains(panel)).toBe(true);
    });

    it("keeps presses, keys and the context menu in the panel away from the card's handlers", () => {
        const { seen } = mountOnCard();
        const field = screen.getByTestId("chip-field");
        const set = screen.getByTestId("chip-set");
        for (const target of [field, set]) {
            fireEvent.pointerDown(target);
            fireEvent.mouseDown(target);
            fireEvent.click(target);
            fireEvent.keyDown(target, { key: "Enter" });
            fireEvent.contextMenu(target);
        }
        expect(seen.pointerdown).not.toHaveBeenCalled();
        expect(seen.mousedown).not.toHaveBeenCalled();
        expect(seen.click).not.toHaveBeenCalled();
        expect(seen.keydown).not.toHaveBeenCalled();
        expect(seen.contextmenu).not.toHaveBeenCalled();
    });

    it("does not let a mousedown in the panel be cancelled by the card, so the field can take focus", () => {
        mountOnCard();
        const cancelled = !fireEvent.mouseDown(screen.getByTestId("chip-field"));
        expect(cancelled).toBe(false);
    });

    it("keeps the click on the chip itself away from the card", () => {
        const { getByTestId, seen } = mountOnCard();
        fireEvent.click(getByTestId("spend-chip-button"));
        expect(seen.click).not.toHaveBeenCalled();
    });
});
