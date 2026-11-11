// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MODEL_UNDONE_META_KEY, readModelUndone, type ModelUndone } from "../model-undone";
import { AgentModelUndoneCard } from "./AgentModelUndoneCard";

afterEach(cleanup);

const undone = (): ModelUndone =>
    readModelUndone({
        [MODEL_UNDONE_META_KEY]: {
            at: 1,
            tried: { route: "openrouter", model: "deepseek/deepseek-v4-pro-0813", label: "DeepSeek V4 Pro" },
            back_on: { route: "", model: "", label: "your plan" },
            status: 400,
            reason: "API Error: 400 history refused",
        },
    })!;

const button = (c: HTMLElement, text: string) => [...c.querySelectorAll("button")].find((b) => b.textContent?.includes(text)) as HTMLButtonElement;

describe("AgentModelUndoneCard", () => {
    it("says what refused the conversation and where the agent is again", () => {
        const { container } = render(() => <AgentModelUndoneCard card={undone} onCompactThenSwitch={vi.fn()} onDismiss={vi.fn()} />);
        const text = container.querySelector("[data-testid=agent-model-undone]")?.textContent ?? "";
        expect(text).toContain("DeepSeek V4 Pro refused this conversation (400); you are back on your plan");
        expect(text).toContain("history refused");
    });

    it("shows nothing when no change was undone", () => {
        const { container } = render(() => <AgentModelUndoneCard card={() => null} onCompactThenSwitch={vi.fn()} onDismiss={vi.fn()} />);
        expect(container.querySelector("[data-testid=agent-model-undone]")).toBeNull();
    });

    it("compacts then switches to the model it tried, and dismisses once that worked", async () => {
        const onCompactThenSwitch = vi.fn().mockResolvedValue(undefined);
        const onDismiss = vi.fn();
        const { container } = render(() => <AgentModelUndoneCard card={undone} onCompactThenSwitch={onCompactThenSwitch} onDismiss={onDismiss} />);
        fireEvent.click(button(container, "Compact, then switch"));
        await waitFor(() => expect(onDismiss).toHaveBeenCalledTimes(1));
        expect(onCompactThenSwitch).toHaveBeenCalledWith(expect.objectContaining({ tried: expect.objectContaining({ model: "deepseek/deepseek-v4-pro-0813" }) }));
    });

    it("keeps the card and says why when the switch could not be done", async () => {
        const onDismiss = vi.fn();
        const { container } = render(() => (
            <AgentModelUndoneCard card={undone} onCompactThenSwitch={() => Promise.reject(new Error("not compacted in time"))} onDismiss={onDismiss} />
        ));
        fireEvent.click(button(container, "Compact, then switch"));
        await waitFor(() => expect(container.querySelector("[role=alert]")?.textContent).toContain("not compacted in time"));
        expect(onDismiss).not.toHaveBeenCalled();
        expect(button(container, "Compact, then switch").disabled).toBe(false);
    });

    it("dismisses on request, and is disabled while it works", async () => {
        const [card] = createSignal<ModelUndone | null>(undone());
        let finish: () => void = () => {};
        const pending = new Promise<void>((r) => (finish = r));
        const onDismiss = vi.fn();
        const { container } = render(() => <AgentModelUndoneCard card={card} onCompactThenSwitch={() => pending} onDismiss={onDismiss} />);
        fireEvent.click(button(container, "Dismiss"));
        expect(onDismiss).toHaveBeenCalledTimes(1);
        fireEvent.click(button(container, "Compact, then switch"));
        await waitFor(() => expect(button(container, "Compacting").disabled).toBe(true));
        expect(button(container, "Dismiss").disabled).toBe(true);
        finish();
        await waitFor(() => expect(onDismiss).toHaveBeenCalledTimes(2));
    });
});
