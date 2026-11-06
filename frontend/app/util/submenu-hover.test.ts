// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Unit tests for the submenu hover-intent core
// (SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10 §5 Phase 1).

import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

import { closePeers, createSubmenuHover, type SubmenuHoverController } from "./submenu-hover";

function makeRect(x: number, y: number, w: number, h: number): DOMRect {
    return {
        x,
        y,
        width: w,
        height: h,
        top: y,
        left: x,
        right: x + w,
        bottom: y + h,
        toJSON: () => ({ x, y, width: w, height: h }),
    } as DOMRect;
}

/** Submenu opens to the RIGHT of its trigger, matching this app's default placement. */
const SUBMENU_RECT = makeRect(300, 100, 160, 200); // x:[300,460] y:[100,300]

function moveTo(x: number, y: number) {
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y }));
}

describe("createSubmenuHover", () => {
    let onOpen: Mock<() => void>;
    let onClose: Mock<() => void>;
    let controller: SubmenuHoverController;

    beforeEach(() => {
        vi.useFakeTimers();
        onOpen = vi.fn<() => void>();
        onClose = vi.fn<() => void>();
        controller = createSubmenuHover({ openDelayMs: 90, closeSafetyTimeoutMs: 300, onOpen, onClose });
    });

    afterEach(() => {
        controller.dispose();
        vi.useRealTimers();
    });

    describe("open delay", () => {
        it("does not open immediately on trigger enter", () => {
            controller.onTriggerEnter();
            expect(onOpen).not.toHaveBeenCalled();
        });

        it("opens after the configured delay of sustained hover", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();
        });

        it("cancels the pending open if the trigger is left before the delay elapses", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(50);
            controller.onTriggerLeave({ clientX: 0, clientY: 0 });
            vi.advanceTimersByTime(100);
            expect(onOpen).not.toHaveBeenCalled();
            expect(onClose).not.toHaveBeenCalled(); // never opened, so nothing to close
        });

        it("re-entering the same trigger while already open does not re-fire onOpen", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();

            controller.onTriggerEnter();
            vi.advanceTimersByTime(200);
            expect(onOpen).toHaveBeenCalledOnce();
        });
    });

    describe("safe-triangle close", () => {
        function openSubmenu() {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();
        }

        it("stays open while the cursor moves diagonally toward the submenu", () => {
            openSubmenu();
            // Leave the trigger row just left of the submenu, vertically centered.
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            // Advance through a path that stays inside the triangle toward (300,150)/(300,250).
            moveTo(270, 195);
            vi.advanceTimersByTime(10);
            moveTo(290, 190);
            vi.advanceTimersByTime(10);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("stays open for a path along the triangle's lower edge, a few pixels past the submenu's bottom", () => {
            openSubmenu();
            // The row's leave point is near the submenu's bottom; the hand heads for its last item and wobbles.
            controller.onTriggerLeave({ clientX: 250, clientY: 280 });
            moveTo(270, 292);
            vi.advanceTimersByTime(10);
            moveTo(290, 305);
            vi.advanceTimersByTime(10);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("still closes for a pointer going straight down from the same point", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 280 });
            moveTo(250, 330);
            vi.advanceTimersByTime(10);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("closes promptly when the cursor moves away from the submenu", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            // Straight up and away — outside the triangle immediately.
            moveTo(250, 0);
            vi.advanceTimersByTime(10);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("stays open indefinitely once the cursor arrives inside the submenu panel", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            controller.onSubmenuEnter();
            // Well past the safety timeout — should never close while "inside".
            vi.advanceTimersByTime(10_000);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("closes after leaving the submenu panel itself", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            controller.onSubmenuEnter();
            controller.onSubmenuLeave({ clientX: 380, clientY: 200 });
            vi.advanceTimersByTime(300);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("falls back to the safety timeout if the cursor never moves after leaving", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            expect(onClose).not.toHaveBeenCalled();
            vi.advanceTimersByTime(299);
            expect(onClose).not.toHaveBeenCalled();
            vi.advanceTimersByTime(1);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("falls back to a plain delay when no submenu geometry has been registered", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();

            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            vi.advanceTimersByTime(300);
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("re-entering the trigger mid-close cancels the close", () => {
            openSubmenu();
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            vi.advanceTimersByTime(100);
            controller.onTriggerEnter();
            vi.advanceTimersByTime(300);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("reads the submenu rect fresh on each move (survives a mid-hover reposition)", () => {
            let rect = SUBMENU_RECT;
            controller.setSubmenuEl({ getBoundingClientRect: () => rect });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);

            // Submenu got shifted further right by autoUpdate mid-hover.
            rect = makeRect(500, 100, 160, 200);
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            // A path toward the OLD rect location would now be outside the triangle
            // for the NEW rect if we were using stale geometry; toward the new
            // location it should stay open.
            moveTo(450, 195);
            vi.advanceTimersByTime(10);
            expect(onClose).not.toHaveBeenCalled();
        });
    });

    describe("close", () => {
        it("closes immediately, bypassing any safe-triangle grace period", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();

            controller.close();
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("cancels a pending open without ever calling onOpen or onClose", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(50);
            controller.close();
            vi.advanceTimersByTime(100);
            expect(onOpen).not.toHaveBeenCalled();
            expect(onClose).not.toHaveBeenCalled();
        });

        it("is a no-op when already closed", () => {
            controller.close();
            expect(onClose).not.toHaveBeenCalled();
        });

        it("leaves the controller reusable for a later onTriggerEnter", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            controller.close();
            onOpen.mockClear();

            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();
        });
    });

    describe("openNow", () => {
        it("opens immediately, bypassing the open delay", () => {
            controller.openNow();
            expect(onOpen).toHaveBeenCalledOnce();
        });

        it("cancels a pending delayed open first (no double onOpen)", () => {
            controller.onTriggerEnter();
            vi.advanceTimersByTime(50); // mid-delay, not yet open
            controller.openNow();
            expect(onOpen).toHaveBeenCalledOnce();
            vi.advanceTimersByTime(100); // the original delayed open, if still pending, would fire here
            expect(onOpen).toHaveBeenCalledOnce();
        });

        it("is a no-op when already open (no double onOpen)", () => {
            controller.openNow();
            controller.openNow();
            expect(onOpen).toHaveBeenCalledOnce();
        });

        // The actual bug this exists to fix (reagent P1, PR #2633): a
        // click-driven open must leave the controller's state truthful so a
        // later mouse-leave still starts the normal close-intent window,
        // instead of onTriggerLeave's `if (!isOpen) return` silently
        // skipping it forever.
        it("a later onTriggerLeave starts the normal close-intent window (unlike opening via external state alone)", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.openNow();
            expect(onOpen).toHaveBeenCalledOnce();

            controller.onTriggerLeave({ clientX: 250, clientY: 200 });
            moveTo(250, 0); // straight away from the submenu — outside the safe triangle
            vi.advanceTimersByTime(10);
            expect(onClose).toHaveBeenCalledOnce();
        });
    });

    describe("dispose", () => {
        it("closes an open submenu immediately, same as close()", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            expect(onOpen).toHaveBeenCalledOnce();

            controller.dispose();
            expect(onClose).toHaveBeenCalledOnce();
        });

        it("removes the mousemove listener and stops pending timers — no further onClose after teardown", () => {
            controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
            controller.onTriggerEnter();
            vi.advanceTimersByTime(90);
            controller.onTriggerLeave({ clientX: 250, clientY: 200 });

            controller.dispose();
            onClose.mockClear();
            moveTo(250, 0);
            vi.advanceTimersByTime(500);
            expect(onClose).not.toHaveBeenCalled();
        });
    });
});

describe("createSubmenuHover with closeOnLeave: false (a submenu inside a menu)", () => {
    let onOpen: Mock<() => void>;
    let onClose: Mock<() => void>;
    let controller: SubmenuHoverController;

    beforeEach(() => {
        vi.useFakeTimers();
        onOpen = vi.fn<() => void>();
        onClose = vi.fn<() => void>();
        controller = createSubmenuHover({ closeOnLeave: false, onOpen, onClose });
        controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
        controller.onTriggerEnter();
        vi.advanceTimersByTime(90);
        expect(onOpen).toHaveBeenCalledOnce();
    });

    afterEach(() => {
        controller.dispose();
        vi.useRealTimers();
    });

    it("stays open when the pointer leaves the trigger away from the submenu", () => {
        controller.onTriggerLeave({ clientX: 150, clientY: 400 });
        moveTo(50, 600);
        vi.advanceTimersByTime(5000);
        expect(onClose).not.toHaveBeenCalled();
    });

    it("stays open when the pointer leaves the submenu itself", () => {
        controller.onSubmenuEnter();
        controller.onSubmenuLeave({ clientX: 470, clientY: 150 });
        moveTo(900, 150);
        vi.advanceTimersByTime(5000);
        expect(onClose).not.toHaveBeenCalled();
    });

    it("still closes on an explicit close (a sibling row hovered, the menu closed)", () => {
        controller.onTriggerLeave({ clientX: 150, clientY: 400 });
        controller.close();
        expect(onClose).toHaveBeenCalledOnce();
    });

    it("does not open when the pointer leaves before the open delay", () => {
        controller.close();
        onOpen.mockClear();
        controller.onTriggerEnter();
        controller.onTriggerLeave({ clientX: 150, clientY: 400 });
        vi.advanceTimersByTime(500);
        expect(onOpen).not.toHaveBeenCalled();
    });
});

describe("closeForPeer: a peer row entered on the way to the open submenu (#24133)", () => {
    // The trigger row is x:[0,300] y:[100,140]; the pointer leaves it through the bottom edge at (200,140), onto the
    // peer row below, heading for the lower part of the submenu at x:[300,460] y:[100,300].
    const LEAVE = { clientX: 200, clientY: 140 };
    const ENTER_PEER = { clientX: 201, clientY: 141 };
    let onOpen: Mock<() => void>;
    let onClose: Mock<() => void>;
    let controller: SubmenuHoverController;

    beforeEach(() => {
        vi.useFakeTimers();
        onOpen = vi.fn<() => void>();
        onClose = vi.fn<() => void>();
        controller = createSubmenuHover({ closeOnLeave: false, onOpen, onClose });
        controller.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
        controller.onTriggerEnter();
        vi.advanceTimersByTime(90);
        controller.onTriggerLeave(LEAVE);
    });

    afterEach(() => {
        controller.dispose();
        vi.useRealTimers();
    });

    it("holds the submenu open while the pointer stays inside the triangle toward it", () => {
        const settled = vi.fn();
        expect(controller.closeForPeer(ENTER_PEER, settled)).toBe(true);
        moveTo(250, 200);
        moveTo(290, 260);
        expect(onClose).not.toHaveBeenCalled();
        expect(settled).not.toHaveBeenCalled();
    });

    it("keeps it open for good once the pointer arrives in the submenu", () => {
        const settled = vi.fn();
        controller.closeForPeer(ENTER_PEER, settled);
        moveTo(290, 260);
        controller.onSubmenuEnter();
        vi.advanceTimersByTime(5000);
        expect(onClose).not.toHaveBeenCalled();
        expect(settled).not.toHaveBeenCalled();
    });

    it("closes and settles once the pointer leaves the triangle", () => {
        const settled = vi.fn();
        controller.closeForPeer(ENTER_PEER, settled);
        moveTo(250, 200);
        moveTo(201, 260); // straight down, away from the submenu
        expect(onClose).toHaveBeenCalledOnce();
        expect(settled).toHaveBeenCalledOnce();
    });

    it("closes and settles at the safety timeout when the pointer stops inside the triangle", () => {
        const settled = vi.fn();
        controller.closeForPeer(ENTER_PEER, settled);
        vi.advanceTimersByTime(299);
        expect(onClose).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(onClose).toHaveBeenCalledOnce();
        expect(settled).toHaveBeenCalledOnce();
    });

    it("keeps holding while the pointer keeps moving inside the triangle, however slowly (a 720 ms crossing)", () => {
        const settled = vi.fn();
        controller.closeForPeer(ENTER_PEER, settled);
        for (let i = 1; i <= 24; i++) {
            vi.advanceTimersByTime(30);
            moveTo(201 + i * 4, 141 + i * 6); // a straight line toward the submenu's lower part, inside the triangle
        }
        expect(onClose).not.toHaveBeenCalled();
        expect(settled).not.toHaveBeenCalled();
        controller.onSubmenuEnter();
        vi.advanceTimersByTime(5000);
        expect(onClose).not.toHaveBeenCalled();
    });

    it("closes once the pointer stops moving inside the triangle for the safety timeout", () => {
        const settled = vi.fn();
        controller.closeForPeer(ENTER_PEER, settled);
        vi.advanceTimersByTime(200);
        moveTo(250, 200);
        vi.advanceTimersByTime(299);
        expect(onClose).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(onClose).toHaveBeenCalledOnce();
        expect(settled).toHaveBeenCalledOnce();
    });

    it("closes at once, without holding, when the peer is entered outside the triangle", () => {
        const settled = vi.fn();
        expect(controller.closeForPeer({ clientX: 100, clientY: 160 }, settled)).toBe(false);
        expect(onClose).toHaveBeenCalledOnce();
        expect(settled).not.toHaveBeenCalled(); // the caller proceeds itself when nothing was held
    });

    it("closes at once when there is no pointer position or no known leave point", () => {
        expect(controller.closeForPeer(undefined, vi.fn())).toBe(false);
        expect(onClose).toHaveBeenCalledOnce();

        const fresh = createSubmenuHover({ closeOnLeave: false, onOpen, onClose });
        fresh.setSubmenuEl({ getBoundingClientRect: () => SUBMENU_RECT });
        fresh.openNow();
        onClose.mockClear();
        expect(fresh.closeForPeer(ENTER_PEER, vi.fn())).toBe(false);
        expect(onClose).toHaveBeenCalledOnce();
    });

    it("does nothing when it is not open", () => {
        controller.close();
        onClose.mockClear();
        expect(controller.closeForPeer(ENTER_PEER, vi.fn())).toBe(false);
        expect(onClose).not.toHaveBeenCalled();
    });

    it("a forced close drops the held peer's settle callback", () => {
        const settled = vi.fn();
        controller.closeForPeer(ENTER_PEER, settled);
        controller.close();
        expect(onClose).toHaveBeenCalledOnce();
        expect(settled).not.toHaveBeenCalled();
    });

    it("a second peer entered while holding replaces the first one's callback", () => {
        const first = vi.fn();
        const second = vi.fn();
        controller.closeForPeer(ENTER_PEER, first);
        expect(controller.closeForPeer({ clientX: 220, clientY: 170 }, second)).toBe(true);
        vi.advanceTimersByTime(300);
        expect(first).not.toHaveBeenCalled();
        expect(second).toHaveBeenCalledOnce();
    });

    it("does not reuse a stale leave point after the pointer returned from the submenu", () => {
        controller.onSubmenuEnter();
        expect(controller.closeForPeer(ENTER_PEER, vi.fn())).toBe(false);
        expect(onClose).toHaveBeenCalledOnce();
    });
});

describe("closePeers", () => {
    it("settles at once when no peer holds, and after the last held peer when some do", () => {
        const make = (held: boolean) => {
            let settle: (() => void) | undefined;
            const peer = {
                closeForPeer: vi.fn((_pt: unknown, cb?: () => void) => {
                    settle = cb;
                    return held;
                }),
            } as unknown as SubmenuHoverController;
            return { peer, fire: () => settle?.() };
        };
        const self = make(false);
        const none = make(false);
        const then1 = vi.fn();
        closePeers([self.peer, none.peer], self.peer, { clientX: 1, clientY: 2 }, then1);
        expect(self.peer.closeForPeer).not.toHaveBeenCalled();
        expect(then1).toHaveBeenCalledOnce();

        const holding = make(true);
        const then2 = vi.fn();
        closePeers([self.peer, holding.peer], self.peer, { clientX: 1, clientY: 2 }, then2);
        expect(then2).not.toHaveBeenCalled();
        holding.fire();
        expect(then2).toHaveBeenCalledOnce();
    });
});
