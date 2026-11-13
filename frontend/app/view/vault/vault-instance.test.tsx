// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const seen = vi.hoisted(() => ({ model: null as null | { setWidth: (px: number) => void } }));
vi.mock("./vault-view", () => ({
    VaultView: (props: { model: { blockId: string; shown: () => boolean; setWidth: (px: number) => void } }) => {
        seen.model = props.model;
        return <div data-testid="vault-view" data-block={props.model.blockId} data-shown={String(props.model.shown())} />;
    },
}));

const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
const [visibility, setVisibility] = createSignal<"active" | "dormant">("active");
const host = () => ({
    blockId: "vault-block",
    meta: blockMeta as any,
    setMeta: vi.fn(() => Promise.resolve()),
    isFocused: () => false,
    visibility,
});

import { create } from "./vault-instance";

describe("the Vault pane", () => {
    afterEach(() => {
        cleanup();
        setBlockMeta({});
        setVisibility("active");
    });

    it("shows the Vault, shown only while its pane is active", () => {
        const instance = createRoot(() => create(host()));
        const Component = instance.component;
        render(() => <Component ctx={host()} />);
        expect(screen.getByTestId("vault-view").dataset.block).toBe("vault-block");
        expect(screen.getByTestId("vault-view").dataset.shown).toBe("true");
        setVisibility("dormant");
        expect(screen.getByTestId("vault-view").dataset.shown).toBe("false");
    });

    it("is titled with the selected section, which a block saved by the old Warden Vault keeps", () => {
        setBlockMeta({ view: "vault", "vault:section": "cards" });
        const instance = createRoot(() => create(host()));
        expect(instance.liveTitle?.().text).toBe("Vault · Cards and banks");
    });

    it("has the Vault import, export and note actions", () => {
        const instance = createRoot(() => create(host()));
        expect(instance.headerActions?.().map((a) => (a as { title?: string }).title)).toEqual([
            "Import into the Vault",
            "Export the Vault",
            "What the Vault may ask you",
        ]);
    });

    it("moves its header buttons into the pane, and shortens its title, as the pane narrows", () => {
        const instance = createRoot(() => create(host()));
        const Component = instance.component;
        render(() => <Component ctx={host()} />);
        const titles = () => instance.headerActions?.().map((a) => (a as { title?: string }).title);
        const setWidth = seen.model!.setWidth;

        // Until the view has measured the pane, and in a wide one: the header as before.
        expect(titles()).toHaveLength(3);
        setWidth(900);
        expect(titles()).toHaveLength(3);
        expect(instance.liveTitle?.().text).toBe("Vault · Personal data");

        // Too narrow for the title and the three buttons: the buttons are in the pane (vault-narrow-actions.tsx).
        setWidth(420);
        expect(titles()).toEqual([]);
        expect(instance.liveTitle?.().text).toBe("Vault · Personal data");

        // Too narrow for "Vault · " as well: the section's short name.
        setWidth(300);
        expect(instance.liveTitle?.().text).toBe("Personal");
        setBlockMeta({ view: "vault", "vault:section": "kept" });
        expect(instance.liveTitle?.().text).toBe("Kept");

        setWidth(900);
        expect(titles()).toHaveLength(3);
        expect(instance.liveTitle?.().text).toBe("Vault · Kept by AgentMux");
    });
});
