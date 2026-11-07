// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const meta = vi.hoisted(() => ({ value: undefined as unknown, written: [] as unknown[] }));
vi.mock("@/app/store/block-atom-cache", () => ({ getBlockMetaKeyAtom: () => () => meta.value }));
vi.mock("@/app/store/block-meta", () => ({
    setBlockMeta: (_id: string, m: Record<string, unknown>) => {
        meta.written.push(m["agent:messages"]);
        meta.value = m["agent:messages"];
    },
}));

import type { DocumentNode } from "@/app/agent-model/node-types";
import {
    createMessagesPanelControls,
    messagesControlsFor,
    parseMessagesState,
    setMessagesEnabled,
} from "./messages-state";

const incoming = (over: object = {}): DocumentNode =>
    ({ type: "jekt_message", id: "j1", direction: "incoming", ...over }) as unknown as DocumentNode;

describe("parseMessagesState", () => {
    it("is off, open and unseen until a pane says otherwise", () => {
        expect(parseMessagesState(undefined)).toEqual({ enabled: false, open: true, seen: 0 });
        expect(parseMessagesState("junk")).toEqual({ enabled: false, open: true, seen: 0 });
    });

    it("takes what is well formed and defaults the rest", () => {
        expect(parseMessagesState({ enabled: true, open: false, seen: 1234 })).toEqual({
            enabled: true,
            open: false,
            seen: 1234,
        });
        expect(parseMessagesState({ enabled: "yes", seen: -5 })).toEqual({ enabled: false, open: true, seen: 0 });
        expect(parseMessagesState({ seen: Number.NaN }).seen).toBe(0);
    });
});

describe("the pane's controls", () => {
    const make = (forced = (_: DocumentNode) => false) => createRoot(() => createMessagesPanelControls("b1", forced));

    it("turns an incoming jekt into a stub only while the panel is on, and never a forced one", () => {
        meta.value = { enabled: false };
        const off = make();
        expect(off.stubbed(incoming())).toBe(false);
        meta.value = { enabled: true };
        const on = make();
        expect(on.stubbed(incoming())).toBe(true);
        expect(on.stubbed(incoming({ direction: "outgoing" }))).toBe(false);
        expect(on.stubbed({ type: "system_note", id: "s" } as unknown as DocumentNode)).toBe(false);
        expect(make(() => true).stubbed(incoming())).toBe(false);
    });

    it("opens the panel and asks it for a message when a stub is clicked", () => {
        meta.value = { enabled: true, open: false };
        meta.written = [];
        const c = make();
        c.showInPanel("j9");
        expect(meta.written.at(-1)).toMatchObject({ enabled: true, open: true });
        expect(c.request()).toMatchObject({ nodeId: "j9" });
    });

    it("shows a stub's message in the overlay of a narrow pane and leaves the docked state alone", () => {
        meta.value = { enabled: true, open: false };
        meta.written = [];
        const c = make();
        c.setNarrow(true);
        c.showInPanel("j9");
        expect(c.overlayOpen()).toBe(true);
        expect(meta.written).toEqual([]);
        expect(c.request()).toMatchObject({ nodeId: "j9" });
    });

    it("does not open the overlay for a stub in a wide pane", () => {
        meta.value = { enabled: true, open: true };
        const c = make();
        c.showInPanel("j9");
        expect(c.overlayOpen()).toBe(false);
    });

    it("is found by pane for what acts on it from outside", () => {
        meta.value = { enabled: true };
        const c = make();
        expect(messagesControlsFor("b1")).toBe(c);
        expect(messagesControlsFor("nope")).toBeUndefined();
    });

    it("keeps the rest of its state when one part changes", () => {
        meta.value = { enabled: true, open: true, seen: 50 };
        const c = make();
        c.patch({ seen: 99 });
        expect(meta.value).toEqual({ enabled: true, open: true, seen: 99 });
    });

    it("is switched on and off from outside, opening when it comes on", () => {
        meta.value = { enabled: false, open: false, seen: 7 };
        setMessagesEnabled("b1", true);
        expect(meta.value).toEqual({ enabled: true, open: true, seen: 7 });
        setMessagesEnabled("b1", false);
        expect(meta.value).toMatchObject({ enabled: false, seen: 7 });
    });
});
