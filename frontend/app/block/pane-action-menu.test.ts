// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal, type JSX } from "solid-js";
import type { SignalAtom } from "@/util/util";
import { afterEach, describe, expect, it, vi } from "vitest";
import { publishMessagesHeaderButton } from "@/app/view/agent/messages/messages-state";
import { paneActionMenuItems, tabWantsYou } from "./pane-action-menu";

const event = { clientX: 1, clientY: 2 } as MouseEvent;
const modelOf = (...buttons: Array<IconButtonDecl | ToggleIconButtonDecl>) => ({ endIconButtons: () => buttons }) as unknown as ViewModel;

/** A toggle as the agent's Find declares it: a fake atom that reads the bar and writes through. */
function toggle(over: Partial<ToggleIconButtonDecl> = {}) {
    const [on, setOn] = createSignal(false);
    const active = (() => on()) as unknown as SignalAtom<boolean>;
    active._set = vi.fn((v: boolean) => setOn(v));
    return { decl: { elemtype: "toggleiconbutton", icon: "magnifying-glass", title: "Find in conversation (Ctrl+F)", active, ...over } as ToggleIconButtonDecl, active, set: setOn };
}

afterEach(() => {
    publishMessagesHeaderButton("b1", null);
});

describe("paneActionMenuItems", () => {
    it("is nothing for a pane with no buttons, and for no view model", () => {
        expect(paneActionMenuItems(null, event)).toEqual([]);
        expect(paneActionMenuItems(modelOf(), event)).toEqual([]);
        expect(paneActionMenuItems({} as ViewModel, event)).toEqual([]);
    });

    it("makes a row of each button, with its icon and words, and a separator under them", () => {
        const click = vi.fn();
        const items = paneActionMenuItems(
            modelOf({ elemtype: "iconbutton", icon: "play", title: "Click to Start Shell", click }),
            event
        );
        expect(items).toHaveLength(2);
        expect(items[0]).toMatchObject({ label: "Click to Start Shell", icon: "play", enabled: true });
        expect(items[1]).toEqual({ type: "separator" });
        items[0].click!();
        expect(click).toHaveBeenCalledWith(event);
    });

    it("makes a toggle a row with its icon that flips it on a pick, as its button did", () =>
        createRoot((dispose) => {
            const t = toggle();
            const [first] = paneActionMenuItems(modelOf(t.decl), event);
            expect(first).toMatchObject({ label: "Find in conversation (Ctrl+F)", icon: "magnifying-glass" });
            expect(first.type).toBeUndefined();
            first.click!();
            expect(t.active._set).toHaveBeenCalledWith(true);
            t.set(true);
            paneActionMenuItems(modelOf(t.decl), event)[0].click!();
            expect(t.active._set).toHaveBeenLastCalledWith(false);
            dispose();
        }));

    it("keeps a disabled button's row, disabled, and leaves out a button with no words or no action", () => {
        const items = paneActionMenuItems(
            modelOf(
                { elemtype: "iconbutton", icon: "x", title: "Off for now", disabled: true },
                { elemtype: "iconbutton", icon: "y", title: "  " },
                { elemtype: "iconbutton", icon: "z", title: "Decoration", noAction: true }
            ),
            event
        );
        expect(items.filter((i) => i.type !== "separator").map((i) => [i.label, i.enabled])).toEqual([["Off for now", false]]);
    });

    it("gives a button whose face is a node its menu icon, or none", () => {
        const [named] = paneActionMenuItems(
            modelOf({ elemtype: "iconbutton", icon: (() => null) as unknown as JSX.Element, menuIcon: "envelope", title: "Messages" }),
            event
        );
        expect(named.icon).toBe("envelope");
        const [row] = paneActionMenuItems(
            modelOf({ elemtype: "iconbutton", icon: (() => null) as unknown as JSX.Element, title: "Messages, 3 unread" }),
            event
        );
        expect(row.label).toBe("Messages, 3 unread");
        expect(row.icon).toBeUndefined();
    });

    it("lists every view's actions in the order the view declared them (the agent's Find, then its Messages)", () => {
        const find = toggle();
        const messages = toggle({ icon: "envelope", title: "Messages" });
        expect(paneActionMenuItems(modelOf(find.decl, messages.decl), event).map((i) => i.label)).toEqual([
            "Find in conversation (Ctrl+F)",
            "Messages",
            undefined,
        ]);
    });
});

describe("tabWantsYou", () => {
    it("is true while the pane's Messages has something for the person, and false once it has not", () =>
        createRoot((dispose) => {
            const [unread, setUnread] = createSignal(0);
            const t = toggle({ icon: "envelope", title: "Messages", attention: () => unread() > 0 });
            expect(tabWantsYou("b1")).toBe(false);
            publishMessagesHeaderButton("b1", t.decl);
            expect(tabWantsYou("b1")).toBe(false);
            setUnread(2);
            expect(tabWantsYou("b1")).toBe(true);
            expect(tabWantsYou("other")).toBe(false);
            setUnread(0);
            expect(tabWantsYou("b1")).toBe(false);
            dispose();
        }));
});
