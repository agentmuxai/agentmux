// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Task #23943, part 3: the toast stack and the dialogs (window, tab and pane scope) follow chrome zoom, as the
 * operator decided. Neither is anchored to anything, so neither uses `AnchoredPopover`; both use its shape: a shell
 * left unzoomed (placed, or centred, in real pixels) around a body that carries `zoom: var(--zoomfactor)`.
 * jsdom loads no stylesheet, so the rules are read from the source.
 */

import { setChromeZoomSignal } from "@/app/store/chrome-zoom";
import { setNotifications } from "@/app/store/flash-notifications";
import { cleanup, render } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NotificationBubbles } from "../notification/notificationbubbles";

/** The declarations of the first rule whose selector is exactly `selector`. */
function ruleOf(file: string, selector: string): string {
    const src = readFileSync(join(__dirname, file), "utf8");
    const at = src.indexOf(`${selector} {`);
    expect(at, `${selector} in ${file}`).toBeGreaterThanOrEqual(0);
    return src.slice(at, src.indexOf("\n}", at));
}

describe("dialogs and chrome zoom", () => {
    it("scales the dialog panel, not the overlay around it", () => {
        expect(ruleOf("modal.scss", ".modal-panel")).toContain("zoom: var(--zoomfactor)");
        expect(ruleOf("modal.scss", ".modal-root")).not.toContain("zoom:");
        expect(ruleOf("modal.scss", ".modal-backdrop")).not.toContain("zoom:");
    });

    it("divides a viewport length by the zoom, so a scaled panel still fits the window", () => {
        expect(ruleOf("modal.scss", ".modal-panel")).toMatch(/max-height: calc\(\(100vh - .*\) \/ var\(--zoomfactor\)\)/);
        const palette = ruleOf("../modals/command-palette.scss", ".modal-panel.command-palette-panel");
        expect(palette).toContain("/ var(--zoomfactor)");
        // Its fixed 480px height is a zoomed length too (720 real px at 150%, off a 750 px window): capped by the
        // window. It drops from the top (the overlay's padding-top is 15vh, its bottom padding 24px): 85vh - 24px.
        expect(palette).toMatch(/max-height: min\(480px, calc\(\(85vh - 24px\) \/ var\(--zoomfactor\)\)\)/);
        expect(readFileSync(join(__dirname, "modal.scss"), "utf8")).toMatch(/\[data-placement="top"\] \{[^}]*padding-top: 15vh/);
        expect(ruleOf("../view/accounts/accounts-gallery.scss", ".modal-panel.accounts-chooser")).toContain("/ var(--zoomfactor)");
    });
});

describe("dialog bodies and chrome zoom", () => {
    it("divides a viewport length in a dialog body by the zoom, like the panel's own cap", () => {
        // Inside the zoomed panel 60vh is 60vh times the zoom in real pixels: at 150% the body outgrows the panel.
        const src = readFileSync(join(__dirname, "../view/bundle/components/BundleImportModal.scss"), "utf8");
        expect(src).toContain("max-height: calc(60vh / var(--zoomfactor))");
        expect(src).not.toMatch(/max-height: 60vh/);
    });
});

describe("toasts and chrome zoom", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });
    afterEach(() => {
        cleanup();
        setNotifications([]);
        setChromeZoomSignal(1);
        document.body.innerHTML = "";
        vi.useRealTimers();
    });

    it("keeps the stack's shell unzoomed and puts the zoom on its body", () => {
        expect(ruleOf("../notification/notificationbubbles.scss", ".notification-bubbles-body")).toContain("zoom: var(--zoomfactor)");
        // The expanded stack's vertical scrollbar takes room from the fixed width: it must not grow a sideways one.
        expect(ruleOf("../notification/notificationbubbles.scss", ".notification-bubbles-body")).toContain("overflow-x: hidden");
        const src = readFileSync(join(__dirname, "../notification/notificationbubbles.scss"), "utf8");
        expect(src).not.toMatch(/\.notification-bubbles \{[^}]*zoom:/);

        setNotifications([{ id: "t", icon: "bell", title: "t", message: "", timestamp: "" }]);
        render(() => <NotificationBubbles />);
        const shell = document.querySelector(".notification-bubbles") as HTMLElement;
        expect(shell.querySelector(":scope > .notification-bubbles-body")).not.toBeNull();
    });

    it("places the stack's gaps in real pixels: the usual ones times the zoom, so it still clears the status bar", () => {
        const stack = () => document.querySelector(".notification-bubbles") as HTMLElement;
        setNotifications([{ id: "t", icon: "bell", title: "t", message: "", timestamp: "" }]);
        render(() => <NotificationBubbles />);
        vi.advanceTimersByTime(50);
        expect([stack().style.right, stack().style.bottom]).toEqual(["58px", "34px"]);

        setChromeZoomSignal(1.5);
        vi.advanceTimersByTime(50);
        expect([stack().style.right, stack().style.bottom]).toEqual(["87px", "51px"]);
    });

    it("keeps clear of the status bar however tall it is", () => {
        // The usual bottom gap is one row of the bar times the zoom; the bar itself is marked, so a taller bar moves
        // the stack up instead of being covered.
        const bar = readFileSync(join(__dirname, "../statusbar/StatusBar.tsx"), "utf8");
        expect(bar).toMatch(/<div class="status-bar" data-toast-avoid ref=/);
    });

    it("opens at the current zoom's gaps when the zoom changed while no toast showed", () => {
        render(() => <NotificationBubbles />);
        setChromeZoomSignal(1.5);
        setNotifications([{ id: "t", icon: "bell", title: "t", message: "", timestamp: "" }]);
        const stack = document.querySelector(".notification-bubbles") as HTMLElement;
        // Before any measure (a frame later): already the 1.5 gaps, not the 1x bottom beside a 1.5 right.
        expect([stack.style.right, stack.style.bottom]).toEqual(["87px", "51px"]);
    });
});
