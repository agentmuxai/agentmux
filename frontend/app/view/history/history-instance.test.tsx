// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./history-manager", () => ({
    HistoryManager: (props: { paneBlockId: string; visible: boolean }) => (
        <div data-testid="history-manager" data-block={props.paneBlockId} data-visible={String(props.visible)} />
    ),
}));

const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
const [visibility, setVisibility] = createSignal<"active" | "dormant">("active");
const host = () => ({
    blockId: "history-block",
    meta: blockMeta as any,
    setMeta: vi.fn(() => Promise.resolve()),
    isFocused: () => false,
    visibility,
});

import { create } from "./history-instance";

describe("the History pane", () => {
    afterEach(() => {
        cleanup();
        setBlockMeta({});
        setVisibility("active");
    });

    it("gives the manager its own block id, visible while the pane is active", () => {
        const instance = createRoot(() => create(host()));
        const Component = instance.component;
        render(() => <Component ctx={host()} />);
        const manager = screen.getByTestId("history-manager");
        expect(manager.dataset.block).toBe("history-block");
        expect(manager.dataset.visible).toBe("true");
        setVisibility("dormant");
        expect(manager.dataset.visible).toBe("false");
    });

    it("zooms the pane with term:zoom", () => {
        setBlockMeta({ "term:zoom": 1.25 });
        const instance = createRoot(() => create(host()));
        const Component = instance.component;
        const { container } = render(() => <Component ctx={host()} />);
        expect((container.querySelector(".history-pane") as HTMLElement).style.zoom).toBe("1.25");
    });
});
