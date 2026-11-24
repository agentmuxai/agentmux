// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { attachStageInput, COPY_DELAY_MS } from "./stage-input";

let unwire: (() => void) | null = null;
afterEach(() => unwire?.());

function setup(mac = false) {
    const stage = document.createElement("div");
    stage.tabIndex = 0;
    const canvas = document.createElement("canvas");
    const button = document.createElement("button");
    stage.append(canvas, button);
    document.body.append(stage);
    const sent: Record<string, unknown>[] = [];
    unwire = attachStageInput(stage, () => canvas, (m) => sent.push(m as Record<string, unknown>), mac);
    return { stage, canvas, button, sent };
}

const key = (target: Element, type: string, init: KeyboardEventInit) => {
    const e = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(e);
    return e;
};

describe("a streamed pane's stage", () => {
    it("sends the keys pressed on the page and keeps them from the browser", () => {
        const { stage, sent } = setup();
        const e = key(stage, "keydown", { key: "a", code: "KeyA" });
        expect(sent).toHaveLength(1);
        expect(sent[0]).toMatchObject({ t: "key", type: "down", key: "a", text: "a" });
        expect(e.defaultPrevented).toBe(true);
    });

    it("leaves a control drawn over the page (Reconnect) its own keys", () => {
        const { button, sent } = setup();
        const e = key(button, "keydown", { key: "Enter", code: "Enter" });
        expect(sent).toHaveLength(0);
        expect(e.defaultPrevented).toBe(false);
        const tab = key(button, "keydown", { key: "Tab", code: "Tab" });
        expect(tab.defaultPrevented).toBe(false);
    });

    it("sends a press on the canvas but not one on a control over it", () => {
        const { canvas, button, sent } = setup();
        const press = (el: Element) => el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, clientX: 5, clientY: 6, button: 0, detail: 1 }));
        press(button);
        expect(sent).toHaveLength(0);
        press(canvas);
        expect(sent[0]).toMatchObject({ t: "mouse", type: "down", button: "left" });
    });
});

// The person's own clipboard reaches the page through the browser's paste event: the chord itself is left alone so that the event fires.
const paste = (target: Element, text: string) => {
    const e = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(e, "clipboardData", { value: { getData: (type: string) => (type === "text/plain" ? text : "") } });
    target.dispatchEvent(e);
    return e;
};

describe("a streamed pane's stage and the person's clipboard", () => {
    it("sends the text of a paste, exactly, as one paste message and keeps it from the browser", () => {
        const { stage, sent } = setup();
        const text = "Hello, World!\nsecond line\t#42 \u00e9";
        const e = paste(stage, text);
        expect(sent).toEqual([{ t: "paste", text }]);
        expect(e.defaultPrevented).toBe(true);
    });

    it("leaves the paste chord alone (no key message, not cancelled) so the browser's paste event fires", () => {
        const { stage, sent } = setup();
        for (const init of [{ ctrlKey: true }, { metaKey: true }]) {
            const e = key(stage, "keydown", { key: "v", code: "KeyV", ...init });
            expect(e.defaultPrevented, "cancelling the chord would cancel the paste").toBe(false);
        }
        expect(sent).toHaveLength(0);
    });

    it("sends nothing for an empty clipboard, and ignores a paste into a control drawn over the page", () => {
        const { stage, button, sent } = setup();
        paste(stage, "");
        paste(button, "not for the page");
        expect(sent).toHaveLength(0);
    });
});

describe("a streamed pane's stage and copying out of the remote browser", () => {
    it("asks for what the browser copied a moment after the person's Ctrl+C or Ctrl+X, and only then", () => {
        vi.useFakeTimers();
        try {
            const { stage, sent } = setup();
            key(stage, "keydown", { key: "c", code: "KeyC", ctrlKey: true });
            expect(sent.map((m) => m.t)).toEqual(["key"]);
            vi.advanceTimersByTime(COPY_DELAY_MS + 1);
            expect(sent.map((m) => m.t)).toEqual(["key", "copy"]);
            key(stage, "keydown", { key: "x", code: "KeyX", ctrlKey: true });
            vi.advanceTimersByTime(COPY_DELAY_MS + 1);
            expect(sent.filter((m) => m.t === "copy")).toHaveLength(2);
            // A plain C, a Ctrl+A and a paste ask for nothing.
            sent.length = 0;
            key(stage, "keydown", { key: "c", code: "KeyC" });
            key(stage, "keydown", { key: "a", code: "KeyA", ctrlKey: true });
            key(stage, "keydown", { key: "v", code: "KeyV", ctrlKey: true });
            vi.advanceTimersByTime(COPY_DELAY_MS * 3);
            expect(sent.filter((m) => m.t === "copy")).toHaveLength(0);
        } finally {
            vi.useRealTimers();
        }
    });

    it("on a Mac, sends Cmd+C and Cmd+X as the remote browser's Ctrl chords and asks for the copy; Cmd+V stays the paste event", () => {
        vi.useFakeTimers();
        try {
            const { stage, sent } = setup(true);
            key(stage, "keydown", { key: "c", code: "KeyC", metaKey: true });
            expect(sent[0]).toMatchObject({ t: "key", key: "c", mods: 2, text: "" });
            vi.advanceTimersByTime(COPY_DELAY_MS + 1);
            expect(sent.map((m) => m.t)).toEqual(["key", "copy"]);
            key(stage, "keydown", { key: "x", code: "KeyX", metaKey: true });
            vi.advanceTimersByTime(COPY_DELAY_MS + 1);
            expect(sent.filter((m) => m.t === "copy")).toHaveLength(2);
            // The release goes out as the same chord; Cmd+V is not a key at all (the paste event carries the text).
            sent.length = 0;
            key(stage, "keyup", { key: "c", code: "KeyC", metaKey: true });
            key(stage, "keydown", { key: "v", code: "KeyV", metaKey: true });
            expect(sent).toHaveLength(1);
            expect(sent[0]).toMatchObject({ t: "key", type: "up", mods: 2 });
        } finally {
            vi.useRealTimers();
        }
    });

    it("off a Mac, Cmd is sent as Meta and copies nothing", () => {
        vi.useFakeTimers();
        try {
            const { stage, sent } = setup(false);
            key(stage, "keydown", { key: "c", code: "KeyC", metaKey: true });
            expect(sent[0]).toMatchObject({ t: "key", mods: 4 });
            vi.advanceTimersByTime(COPY_DELAY_MS * 3);
            expect(sent.filter((m) => m.t === "copy")).toHaveLength(0);
        } finally {
            vi.useRealTimers();
        }
    });

    it("does not ask after the stage is taken down", () => {
        vi.useFakeTimers();
        try {
            const { stage, sent } = setup();
            key(stage, "keydown", { key: "c", code: "KeyC", ctrlKey: true });
            unwire?.();
            unwire = null;
            vi.advanceTimersByTime(COPY_DELAY_MS * 3);
            expect(sent.filter((m) => m.t === "copy")).toHaveLength(0);
        } finally {
            vi.useRealTimers();
        }
    });
});
