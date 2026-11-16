// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { addressToUrl, clampSide } from "./use-streamed-pane";
import { buttonName, frameBytes, isPasteChord, keyMessage, modifiers, mouseMessage, parseServerMessage, typedText, wheelMessage } from "./streamed-protocol";

const none = { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false };

describe("frames and server messages", () => {
    it("takes the JPEG out of a frame message and ignores other kinds", () => {
        expect(Array.from(frameBytes(new Uint8Array([1, 0xff, 0xd8]).buffer)!)).toEqual([0xff, 0xd8]);
        expect(frameBytes(new Uint8Array([2, 0xff]).buffer)).toBeNull();
        expect(frameBytes(new Uint8Array([1]).buffer)).toBeNull();
    });

    it("reads the messages srv sends and drops what it does not know", () => {
        expect(parseServerMessage('{"t":"nav","url":"https://a.test/","title":"A","canGoBack":true,"canGoForward":false,"loading":true}')).toEqual({
            t: "nav",
            url: "https://a.test/",
            title: "A",
            canGoBack: true,
            canGoForward: false,
            loading: true,
        });
        expect(parseServerMessage('{"t":"size","w":800,"h":600}')).toEqual({ t: "size", w: 800, h: 600 });
        expect(parseServerMessage('{"t":"error","message":"busy"}')).toEqual({ t: "error", message: "busy" });
        for (const bad of ["", "nonsense", "null", '{"t":"size","w":"a"}', '{"t":"launch"}']) expect(parseServerMessage(bad)).toBeNull();
    });
});

describe("what the page sends", () => {
    it("measures a mouse position from the canvas and names the button", () => {
        const m = mouseMessage("down", { ...none, clientX: 130, clientY: 90, button: 2, buttons: 2, detail: 1 }, { left: 100, top: 50 });
        expect(m).toMatchObject({ t: "mouse", type: "down", x: 30, y: 40, button: "right", buttons: 2, clicks: 1 });
        const move = mouseMessage("move", { ...none, clientX: 5, clientY: 5, button: 0, buttons: 0, detail: 0 }, { left: 0, top: 0 });
        expect(move).toMatchObject({ button: "none", clicks: 0 });
        expect([buttonName(0), buttonName(1), buttonName(2), buttonName(3)]).toEqual(["left", "middle", "right", "none"]);
    });

    it("turns a wheel in lines or pages into pixels", () => {
        const w = (deltaMode: number) => wheelMessage({ ...none, clientX: 10, clientY: 10, deltaX: 0, deltaY: 3, deltaMode }, { left: 0, top: 0 }, 500);
        expect(w(0).dy).toBe(3);
        expect(w(1).dy).toBe(48);
        expect(w(2).dy).toBe(1500);
    });

    it("types a character, Enter as a carriage return, and nothing for a chord or a special key", () => {
        expect(typedText({ key: "a", ctrlKey: false, metaKey: false })).toBe("a");
        expect(typedText({ key: "é", ctrlKey: false, metaKey: false })).toBe("é");
        expect(typedText({ key: "Enter", ctrlKey: false, metaKey: false })).toBe("\r");
        expect(typedText({ key: "ArrowLeft", ctrlKey: false, metaKey: false })).toBe("");
        expect(typedText({ key: "a", ctrlKey: true, metaKey: false })).toBe("");
        expect(keyMessage("down", { ...none, shiftKey: true, key: "A", code: "KeyA", keyCode: 65 })).toMatchObject({ t: "key", text: "A", vk: 65, mods: 8 });
    });

    it("leaves a paste chord to the browser's own paste event", () => {
        expect(isPasteChord({ key: "v", ctrlKey: true, metaKey: false })).toBe(true);
        expect(isPasteChord({ key: "V", ctrlKey: false, metaKey: true })).toBe(true);
        expect(isPasteChord({ key: "v", ctrlKey: false, metaKey: false })).toBe(false);
        expect(modifiers({ altKey: true, ctrlKey: true, metaKey: true, shiftKey: true })).toBe(15);
    });
});

describe("the pane's own numbers and address", () => {
    it("keeps a size inside what the server accepts", () => {
        expect(clampSide(50)).toBe(100);
        expect(clampSide(800.7)).toBe(800);
        expect(clampSide(9000)).toBe(4096);
    });

    it("writes a bare address as https and leaves one with a scheme, or a host and port, alone", () => {
        expect(addressToUrl("example.com")).toBe("https://example.com");
        expect(addressToUrl("  example.com/a b ")).toBe("https://example.com/a b");
        expect(addressToUrl("http://127.0.0.1:8080/x")).toBe("http://127.0.0.1:8080/x");
        expect(addressToUrl("localhost:3000")).toBe("https://localhost:3000");
        expect(addressToUrl("file:///etc/passwd")).toBe("file:///etc/passwd");
        expect(addressToUrl("")).toBe("");
    });
});
