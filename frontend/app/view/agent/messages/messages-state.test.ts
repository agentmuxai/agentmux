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
import { deriveDisplayNodes, isForcedRow, VIEW_PRESETS } from "../virtualization/verbosity";
import {
    createMessagesPanelControls,
    DEFAULT_MESSAGES_STATE,
    messagesControlsFor,
    parseMessagesState,
    setMessagesEnabled,
    showInPanelIfHeld,
    withoutPanelJekts,
} from "./messages-state";

const incoming = (over: object = {}): DocumentNode =>
    ({ type: "jekt_message", id: "j1", direction: "incoming", ...over }) as unknown as DocumentNode;

describe("parseMessagesState", () => {
    it("is on, collapsed and unseen until a pane says otherwise", () => {
        expect(parseMessagesState(undefined)).toEqual({ enabled: true, open: false, seen: 0 });
        expect(parseMessagesState("junk")).toEqual({ enabled: true, open: false, seen: 0 });
        expect(DEFAULT_MESSAGES_STATE).toEqual({ enabled: true, open: false, seen: 0 });
    });

    it("takes what is well formed and defaults the rest, and never reads the panel as open from the meta", () => {
        expect(parseMessagesState({ enabled: false, open: true, seen: 1234 })).toEqual({
            enabled: false,
            open: false,
            seen: 1234,
        });
        expect(parseMessagesState({ enabled: "yes", seen: -5 })).toEqual({ enabled: true, open: false, seen: 0 });
        expect(parseMessagesState({ seen: Number.NaN }).seen).toBe(0);
    });
});

describe("the pane's controls", () => {
    const make = (forced = (_: DocumentNode) => false) => createRoot(() => createMessagesPanelControls("b1", forced));

    it("holds an incoming jekt in the panel only while the panel is on, and never a forced one", () => {
        meta.value = { enabled: false };
        const off = make();
        expect(off.hidden(incoming())).toBe(false);
        meta.value = undefined;
        const on = make();
        expect(on.hidden(incoming())).toBe(true);
        expect(on.hidden(incoming({ direction: "outgoing" }))).toBe(false);
        expect(on.hidden({ type: "system_note", id: "s" } as unknown as DocumentNode)).toBe(false);
        expect(make(() => true).hidden(incoming())).toBe(false);
    });

    it("opens the panel on a message it is asked for, for this session only", () => {
        meta.value = { enabled: true };
        meta.written = [];
        const c = make();
        expect(c.state().open).toBe(false);
        c.showInPanel("j9");
        expect(c.state().open).toBe(true);
        expect(meta.written).toEqual([]);
        expect(c.request()).toMatchObject({ nodeId: "j9" });
        c.patch({ open: false });
        expect(c.state().open).toBe(false);
        expect(meta.written).toEqual([]);
    });

    it("is found by pane for what acts on it from outside", () => {
        meta.value = { enabled: true };
        const c = make();
        expect(messagesControlsFor("b1")).toBe(c);
        expect(messagesControlsFor("nope")).toBeUndefined();
    });

    it("keeps the rest of its state when one part changes, and stores no open flag", () => {
        meta.value = { enabled: true, seen: 50 };
        const c = make();
        c.patch({ seen: 99 });
        expect(meta.value).toEqual({ enabled: true, seen: 99 });
        c.patch({ open: true, seen: 120 });
        expect(meta.value).toEqual({ enabled: true, seen: 120 });
        expect(c.state().open).toBe(true);
    });

    it("is switched on and off from outside, keeping what was seen", () => {
        meta.value = { enabled: false, seen: 7 };
        setMessagesEnabled("b1", true);
        expect(meta.value).toEqual({ enabled: true, seen: 7 });
        setMessagesEnabled("b1", false);
        expect(meta.value).toEqual({ enabled: false, seen: 7 });
    });
});

describe("what the conversation leaves to the panel", () => {
    const j = (id: string, over: object = {}) => incoming({ id, ...over });
    const nodes = [j("a"), j("b", { direction: "outgoing" }), j("c"), { type: "system_note", id: "s" } as unknown as DocumentNode];

    it("drops the incoming jekts the panel holds, keeping forced ones and every other row", () => {
        meta.value = { enabled: true };
        expect(withoutPanelJekts(nodes, "b1", () => false).map((n) => n.id)).toEqual(["b", "s"]);
        expect(withoutPanelJekts(nodes, "b1", (n) => n.id === "c").map((n) => n.id)).toEqual(["b", "c", "s"]);
    });

    it("leaves no run line for the jekts it took, in a view that folds jekts away", () => {
        meta.value = { enabled: true };
        const jekt = (id: string) => j(id, { tier: "coord", trust: "host-verified" });
        const shown = deriveDisplayNodes(
            withoutPanelJekts([jekt("a"), jekt("c")], "b1", isForcedRow),
            VIEW_PRESETS.replies,
            { pinnedNodes: new Set(), heldOpenNodes: new Set() }
        );
        expect(shown).toEqual([]);
        // The same view with the panel off folds them into one run line, as before.
        meta.value = { enabled: false };
        const off = deriveDisplayNodes(withoutPanelJekts([jekt("a"), jekt("c")], "b1", isForcedRow), VIEW_PRESETS.replies, {
            pinnedNodes: new Set(),
            heldOpenNodes: new Set(),
        });
        expect(off.map((n) => n.type)).toEqual(["hidden_run"]);
    });

    it("keeps every row while the panel is off for the pane", () => {
        meta.value = { enabled: false };
        expect(withoutPanelJekts(nodes, "b1", () => false)).toBe(nodes);
    });

    it("shows a message in the panel only when the conversation has no row for it", () => {
        meta.value = { enabled: true };
        const c = createRoot(() => createMessagesPanelControls("b1", () => false));
        expect(showInPanelIfHeld("b1", nodes, "a")).toBe(true);
        expect(c.state().open).toBe(true);
        expect(c.request()).toMatchObject({ nodeId: "a" });
        c.patch({ open: false });
        expect(showInPanelIfHeld("b1", nodes, "b")).toBe(false);
        expect(showInPanelIfHeld("b1", nodes, "gone")).toBe(false);
        expect(showInPanelIfHeld("nope", nodes, "a")).toBe(false);
        expect(c.state().open).toBe(false);
    });
});
