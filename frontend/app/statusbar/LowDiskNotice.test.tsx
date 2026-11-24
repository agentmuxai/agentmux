// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The low-disk panel: what it says, and what its buttons and its dismissal do. The floating shell is a stand-in. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ opened: [] as string[] }));

vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (p: {
        role?: JSX.HTMLAttributes<HTMLDivElement>["role"];
        class?: string;
        onDismiss?: (r: string) => void;
        children?: JSX.Element;
    }) => (
        <div data-stand="AnchoredPopover" role={p.role} class={p.class}>
            <button type="button" data-stand="outside" onClick={() => p.onDismiss?.("outside")} />
            {p.children}
        </div>
    ),
}));
vi.mock("@/app/store/block-component-registry", () => ({
    openOrFocusPaneByView: async (view: string) => {
        h.opened.push(view);
    },
}));

import { LowDiskNotice } from "./LowDiskNotice";

afterEach(() => {
    cleanup();
    h.opened.length = 0;
});

const reading = { freeGb: 9.5, totalGb: 100, freePct: 9.5 };

function mount(level: "warn" | "critical", onDismiss = vi.fn()) {
    const view = render(() => <LowDiskNotice anchor={null} notice={{ level, reading, label: "/data" }} onDismiss={onDismiss} />);
    return { ...view, onDismiss };
}

describe("LowDiskNotice", () => {
    it("is an alert in a panel, not a dialog, and says how much is free, where, and how to make room", () => {
        const { container } = mount("warn");
        expect(container.querySelector("[role='alert']")).not.toBeNull();
        expect(container.querySelector("[role='dialog'], [aria-modal]")).toBeNull();
        const text = container.textContent ?? "";
        expect(text).toContain("Disk space is running low");
        expect(text).toContain("9.5 GB free on /data, 9.5% of 100 GB.");
        expect(text).toContain("Delete files you no longer need");
        expect(text).toContain("Cloud Console");
    });

    it("is worded and marked for critical", () => {
        const { container } = mount("critical");
        expect(container.textContent).toContain("Disk space is almost gone");
        expect(container.querySelector("[data-level='critical']")).not.toBeNull();
    });

    it("Dismiss and a click outside both dismiss it", () => {
        const a = mount("warn");
        fireEvent.click(a.getByText("Dismiss"));
        expect(a.onDismiss).toHaveBeenCalledTimes(1);
        const b = mount("warn");
        fireEvent.click(b.container.querySelector("[data-stand='outside']")!);
        expect(b.onDismiss).toHaveBeenCalledTimes(1);
    });

    it("Open Storage dismisses it and opens the Audit pane", () => {
        const { getByText, onDismiss } = mount("warn");
        fireEvent.click(getByText("Open Storage"));
        expect(onDismiss).toHaveBeenCalledTimes(1);
        expect(h.opened).toEqual(["audit"]);
    });
});
