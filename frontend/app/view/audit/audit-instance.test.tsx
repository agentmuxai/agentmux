// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./audit-events-section", () => ({
    default: (props: { visible: boolean; paneBlockId: string }) => <div data-testid="events" data-visible={String(props.visible)} data-pane={props.paneBlockId} />,
}));
vi.mock("./conversations/conversations-section", () => {
    return { default: (props: { visible: boolean }) => <div data-testid="conversations" data-visible={String(props.visible)} /> };
});
vi.mock("./typed/typed-section", () => {
    return { default: (props: { visible: boolean }) => <div data-testid="typed" data-visible={String(props.visible)} /> };
});
vi.mock("./storage/storage-section", () => {
    return { default: (props: { visible: boolean }) => <div data-testid="storage" data-visible={String(props.visible)} /> };
});

const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
const [visibility, setVisibility] = createSignal<"active" | "dormant">("active");
const setMeta = vi.fn((patch: Record<string, unknown>) => {
    setBlockMeta({ ...blockMeta(), ...patch });
    return Promise.resolve();
});
const host = () => ({
    blockId: "audit-block",
    meta: blockMeta as any,
    setMeta,
    isFocused: () => false,
    visibility,
});

import { create } from "./audit-instance";

const mount = () => {
    const instance = createRoot(() => create(host()));
    const Component = instance.component;
    return render(() => <Component ctx={host()} />);
};

describe("the Audit pane", () => {
    afterEach(() => {
        cleanup();
        setBlockMeta({});
        setVisibility("active");
        setMeta.mockClear();
    });

    it("opens on Events and hands it its block", async () => {
        mount();
        const events = await screen.findByTestId("events");
        expect(events.dataset.pane).toBe("audit-block");
        expect(screen.getByRole("tab", { name: "Events" }).getAttribute("aria-selected")).toBe("true");
    });

    it("has the four sections in order", () => {
        mount();
        expect(screen.getAllByRole("tab").map((t) => t.textContent?.trim())).toEqual(["Events", "Conversations", "Typed", "Storage"]);
    });

    it("builds a section only when it is first shown, then keeps it mounted", async () => {
        mount();
        await screen.findByTestId("events");
        expect(screen.queryByTestId("conversations")).toBeNull();
        fireEvent.click(screen.getByRole("tab", { name: "Conversations" }));
        expect(setMeta).toHaveBeenCalledWith({ "audit:section": "conversations" });
        const conversations = await screen.findByTestId("conversations");
        expect(conversations.dataset.visible).toBe("true");
        expect(screen.getByTestId("events").dataset.visible).toBe("false");
        // Going back leaves Conversations mounted, not visible.
        setBlockMeta({ "audit:section": "events" });
        expect(screen.getByTestId("conversations").dataset.visible).toBe("false");
        expect(screen.getByTestId("events").dataset.visible).toBe("true");
    });

    it("opens on the section the meta names", async () => {
        setBlockMeta({ "audit:section": "storage" });
        mount();
        const storage = await screen.findByTestId("storage");
        expect(storage.dataset.visible).toBe("true");
        expect(screen.queryByTestId("events")).toBeNull();
    });

    it("reads and polls only while the pane is active", async () => {
        mount();
        expect((await screen.findByTestId("events")).dataset.visible).toBe("true");
        setVisibility("dormant");
        expect(screen.getByTestId("events").dataset.visible).toBe("false");
    });

    it("shows Events when the meta starts pointing at an agent's events", async () => {
        setBlockMeta({ "audit:section": "typed" });
        mount();
        await screen.findByTestId("typed");
        setBlockMeta({ "audit:section": "typed", "audit:agent": "korp" });
        await waitFor(() => expect(setMeta).toHaveBeenCalledWith({ "audit:section": "events" }));
    });

    it("lets the person pick another section while an agent's events stay in the meta", async () => {
        setBlockMeta({ "audit:agent": "korp" });
        mount();
        await screen.findByTestId("events");
        fireEvent.click(screen.getByRole("tab", { name: "Conversations" }));
        await screen.findByTestId("conversations");
        await new Promise((r) => setTimeout(r, 20));
        expect(setMeta).not.toHaveBeenCalledWith({ "audit:section": "events" });
        expect(screen.getByRole("tab", { name: "Conversations" }).getAttribute("aria-selected")).toBe("true");
    });

    it("zooms with term:zoom", async () => {
        setBlockMeta({ "term:zoom": 1.1 });
        const { container } = mount();
        await screen.findByTestId("events");
        expect((container.querySelector(".section-pane-view") as HTMLElement).style.zoom).toBe("1.1");
    });
});
