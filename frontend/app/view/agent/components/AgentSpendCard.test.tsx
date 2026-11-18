// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SPEND_STOPPED_META_KEY, SPEND_TODAY_META_KEY, readSpendStopped, type SpendCard } from "../spend-stopped";
import { AgentSpendCard } from "./AgentSpendCard";
import { SpendChip } from "./SpendChip";

// The popover's placement and the panel's own calls are tested apart; here only that the chip opens and closes it.
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (p: { children: unknown; onDismiss?: () => void; "aria-label"?: string }) => (
        <div role="dialog" aria-label={p["aria-label"]} data-testid="chip-popover">
            {p.children as never}
        </div>
    ),
}));
vi.mock("./AgentSpendPanel", () => ({
    AgentSpendPanel: (p: { blockId?: string; agentName: string; onClose?: () => void }) => (
        <div data-testid="chip-panel" data-block={p.blockId} data-name={p.agentName}>
            <button onClick={() => p.onClose?.()}>Close</button>
        </div>
    ),
}));

afterEach(cleanup);

const card = (over: Record<string, unknown> = {}): SpendCard =>
    readSpendStopped({ [SPEND_STOPPED_META_KEY]: { at: Date.now(), scope: "agent", cap: 20, spent: 20.1, route: "openrouter", place: "OpenRouter", ...over } })!;
const button = (c: HTMLElement, text: string) => [...c.querySelectorAll("button")].find((b) => b.textContent?.includes(text)) as HTMLButtonElement;
const none = () => null;

describe("AgentSpendCard", () => {
    it("says the agent is stopped and offers Raise, Reset today and Dismiss", () => {
        const { container } = render(() => <AgentSpendCard stopped={() => card()} warning={none} onRaise={vi.fn()} onReset={vi.fn()} onDismiss={vi.fn()} />);
        const text = container.querySelector("[data-testid=agent-spend-stopped]")?.textContent ?? "";
        expect(text).toContain("Stopped: this agent used $20.10 of its $20 today.");
        expect(button(container, "Raise to $40")).toBeTruthy();
        expect(button(container, "Reset today")).toBeTruthy();
        expect(button(container, "Dismiss")).toBeTruthy();
    });

    it("raises to double the cap, resets, and dismisses for this stop", async () => {
        const onRaise = vi.fn().mockResolvedValue(undefined);
        const onReset = vi.fn().mockResolvedValue(undefined);
        const onDismiss = vi.fn();
        const { container } = render(() => <AgentSpendCard stopped={() => card()} warning={none} onRaise={onRaise} onReset={onReset} onDismiss={onDismiss} />);
        fireEvent.click(button(container, "Raise to $40"));
        await waitFor(() => expect(onRaise).toHaveBeenCalledWith(40));
        fireEvent.click(button(container, "Reset today"));
        await waitFor(() => expect(onReset).toHaveBeenCalled());
        fireEvent.click(button(container, "Dismiss"));
        expect(onDismiss).toHaveBeenCalledWith(expect.objectContaining({ at: expect.any(Number) }));
    });

    it("shows the reason when srv refuses", async () => {
        const onRaise = vi.fn().mockRejectedValue(new Error("no agent runs in block b"));
        const { container } = render(() => <AgentSpendCard stopped={() => card()} warning={none} onRaise={onRaise} onReset={vi.fn()} onDismiss={vi.fn()} />);
        fireEvent.click(button(container, "Raise to $40"));
        await waitFor(() => expect(container.textContent).toContain("no agent runs in block b"));
    });

    it("offers only Dismiss when the route's cap was the one reached", () => {
        const { container } = render(() => (
            <AgentSpendCard stopped={() => card({ scope: "route", cap: 100 })} warning={none} onRaise={vi.fn()} onReset={vi.fn()} onDismiss={vi.fn()} />
        ));
        expect(button(container, "Raise")).toBeUndefined();
        expect(button(container, "Reset today")).toBeUndefined();
        expect(button(container, "Dismiss")).toBeTruthy();
        expect(container.textContent).toContain("raised in Settings");
    });

    it("shows a single line from 80% and nothing when there is neither", () => {
        const { container } = render(() => <AgentSpendCard stopped={none} warning={() => card({ spent: 16.5 })} onRaise={vi.fn()} onReset={vi.fn()} onDismiss={vi.fn()} />);
        expect(container.querySelector("[data-testid=agent-spend-warning]")?.textContent).toContain("$16.50 of $20 used today");
        expect(container.querySelectorAll("button").length).toBe(0);
        cleanup();
        const empty = render(() => <AgentSpendCard stopped={none} warning={none} onRaise={vi.fn()} onReset={vi.fn()} onDismiss={vi.fn()} />);
        expect(empty.container.textContent).toBe("");
    });
});

describe("SpendChip", () => {
    const now = () => 1_700_000_000_000;
    const meta = (spent: number, cap: number | null) => ({ [SPEND_TODAY_META_KEY]: { day: Math.floor(now() / 86_400_000), spent, cap } });

    it("shows today's figure against the cap", () => {
        const { container } = render(() => <SpendChip meta={meta(4.2, 20)} now={now} />);
        expect(container.querySelector("[data-testid=spend-chip]")?.textContent).toBe("$4.20 of $20 today");
    });

    it("shows nothing for an agent with no figure for today", () => {
        const { container } = render(() => <SpendChip meta={{}} now={now} />);
        expect(container.textContent).toBe("");
        cleanup();
        const old = render(() => <SpendChip meta={{ [SPEND_TODAY_META_KEY]: { day: 1, spent: 4, cap: 20 } }} now={now} />);
        expect(old.container.textContent).toBe("");
    });

    it("is only a figure with no pane, and a button that opens the Spend cap panel with one", () => {
        const plain = render(() => <SpendChip meta={meta(4.2, 50)} now={now} />);
        expect(plain.container.querySelector("[data-testid=spend-chip-button]")).toBeNull();
        expect(plain.container.querySelector("[title]")?.getAttribute("title")).not.toContain("Click");
        cleanup();

        const { container } = render(() => <SpendChip meta={meta(4.2, 50)} now={now} blockId="blk-1" agentName="AgentY" />);
        const chip = container.querySelector("[data-testid=spend-chip-button]") as HTMLButtonElement;
        expect(chip.getAttribute("aria-expanded")).toBe("false");
        expect(container.querySelector("[title]")?.getAttribute("title")).toContain("model route or an API key");
        expect(container.querySelector("[title]")?.getAttribute("title")).toContain("Click to change its cap.");
        expect(document.querySelector("[data-testid=chip-panel]")).toBeNull();

        fireEvent.click(chip);
        const panel = document.querySelector("[data-testid=chip-panel]") as HTMLElement;
        expect(panel.dataset.block).toBe("blk-1");
        expect(panel.dataset.name).toBe("AgentY");
        expect(chip.getAttribute("aria-expanded")).toBe("true");

        fireEvent.click(chip);
        expect(document.querySelector("[data-testid=chip-panel]")).toBeNull();
        fireEvent.click(chip);
        fireEvent.click(document.querySelector("[data-testid=chip-panel] button") as HTMLButtonElement);
        expect(document.querySelector("[data-testid=chip-panel]")).toBeNull();
    });

    it("does not pass the click on to the card under it", () => {
        const onCard = vi.fn();
        const { container } = render(() => (
            <div onClick={onCard}>
                <SpendChip meta={meta(4.2, 50)} now={now} blockId="blk-1" agentName="AgentY" />
            </div>
        ));
        fireEvent.click(container.querySelector("[data-testid=spend-chip-button]") as HTMLElement);
        expect(onCard).not.toHaveBeenCalled();
    });
});
