// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./audit-manager", () => ({
    AuditManager: (props: { visible?: boolean }) => <div data-testid="audit-manager" data-visible={String(props.visible)} />,
}));

const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
const [visibility, setVisibility] = createSignal<"active" | "dormant">("active");
const host = () => ({
    blockId: "audit-block",
    meta: blockMeta as any,
    setMeta: vi.fn(() => Promise.resolve()),
    isFocused: () => false,
    visibility,
});

import { create } from "./audit-instance";

describe("the Audit pane", () => {
    afterEach(() => {
        cleanup();
        setBlockMeta({});
        setVisibility("active");
    });

    it("polls only while the pane is active", () => {
        const instance = createRoot(() => create(host()));
        const Component = instance.component;
        render(() => <Component ctx={host()} />);
        expect(screen.getByTestId("audit-manager").dataset.visible).toBe("true");
        setVisibility("dormant");
        expect(screen.getByTestId("audit-manager").dataset.visible).toBe("false");
    });

    it("carries the container the feed width queries answer, and zooms with term:zoom", () => {
        setBlockMeta({ "term:zoom": 1.1 });
        const instance = createRoot(() => create(host()));
        const Component = instance.component;
        const { container } = render(() => <Component ctx={host()} />);
        expect(container.querySelector(".audit-container .audit-pane")).not.toBeNull();
        expect((container.querySelector(".audit-pane") as HTMLElement).style.zoom).toBe("1.1");
    });
});
