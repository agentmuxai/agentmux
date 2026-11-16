// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The stage of a streamed pane takes the person's mouse, wheel, keyboard and paste and sends them to the page.
// Keys and paste are taken only while the stage has focus (it is focusable and takes focus when pressed), so typing
// elsewhere in the window is never sent. docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4, §7.

import { isPasteChord, keyMessage, mouseMessage, wheelMessage } from "./streamed-protocol";

type Send = (message: object) => void;

/** Wire input from `stage` (focusable) to `send`, measuring positions on `canvas`. Returns the way to unwire it. */
export function attachStageInput(stage: HTMLElement, canvas: () => HTMLElement | undefined, send: Send): () => void {
    const box = () => (canvas() ?? stage).getBoundingClientRect();
    // Only the page itself (the stage or its canvas) is the page's: a control drawn over it (Reconnect) keeps its own keys and clicks.
    const onPage = (e: Event) => e.target === stage || e.target === canvas();
    let pressed = false;
    let moveFrame = 0;
    let lastMove: MouseEvent | null = null;

    const onDown = (e: MouseEvent) => {
        if (!onPage(e)) return;
        stage.focus();
        pressed = true;
        send(mouseMessage("down", e, box()));
        window.addEventListener("mouseup", onUp);
    };
    const onUp = (e: MouseEvent) => {
        if (!pressed) return;
        pressed = false;
        window.removeEventListener("mouseup", onUp);
        send(mouseMessage("up", e, box()));
    };
    // Moves are sent at most once a frame, the newest one.
    const onMove = (e: MouseEvent) => {
        lastMove = e;
        if (moveFrame) return;
        moveFrame = requestAnimationFrame(() => {
            moveFrame = 0;
            if (lastMove) send(mouseMessage("move", lastMove, box()));
            lastMove = null;
        });
    };
    const onWheel = (e: WheelEvent) => {
        if (!onPage(e)) return;
        e.preventDefault();
        send(wheelMessage(e, box(), box().height));
    };
    const onKey = (type: "down" | "up") => (e: KeyboardEvent) => {
        if (!onPage(e) || isPasteChord(e)) return;
        e.preventDefault();
        send(keyMessage(type, e));
    };
    const onKeyDown = onKey("down");
    const onKeyUp = onKey("up");
    const onPaste = (e: ClipboardEvent) => {
        if (!onPage(e)) return;
        const text = e.clipboardData?.getData("text/plain") ?? "";
        e.preventDefault();
        if (text) send({ t: "paste", text });
    };
    const noMenu = (e: Event) => e.preventDefault();

    stage.addEventListener("mousedown", onDown);
    stage.addEventListener("mousemove", onMove);
    stage.addEventListener("wheel", onWheel, { passive: false });
    stage.addEventListener("keydown", onKeyDown);
    stage.addEventListener("keyup", onKeyUp);
    stage.addEventListener("paste", onPaste);
    stage.addEventListener("contextmenu", noMenu);
    return () => {
        stage.removeEventListener("mousedown", onDown);
        stage.removeEventListener("mousemove", onMove);
        stage.removeEventListener("wheel", onWheel);
        stage.removeEventListener("keydown", onKeyDown);
        stage.removeEventListener("keyup", onKeyUp);
        stage.removeEventListener("paste", onPaste);
        stage.removeEventListener("contextmenu", noMenu);
        window.removeEventListener("mouseup", onUp);
        if (moveFrame) cancelAnimationFrame(moveFrame);
    };
}
