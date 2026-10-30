// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./audit-manager", () => ({
    AuditManager: (props: { visible?: boolean; agent?: string; paneBlockId: string }) => (
        <div data-testid="audit-manager" data-visible={String(props.visible)} data-agent={props.agent ?? ""} data-pane={props.paneBlockId} />
    ),
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

    it("hands the manager its block and the agent named by audit:agent", () => {
        setBlockMeta({ "audit:agent": "korp" });
        const instance = createRoot(() => create(host()));
        const Component = instance.component;
        render(() => <Component ctx={host()} />);
        const manager = screen.getByTestId("audit-manager");
        expect(manager.dataset.pane).toBe("audit-block");
        expect(manager.dataset.agent).toBe("korp");
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
