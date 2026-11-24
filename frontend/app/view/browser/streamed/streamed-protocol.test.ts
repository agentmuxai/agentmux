// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { addressToUrl, clampSide } from "./use-streamed-pane";
import { buttonName, CLIPBOARD_MAX, frameBytes, isCopyChord, isPasteChord, keyMessage, LABEL_MAX, modifiers, mouseMessage, parseServerMessage, plainText, remoteModifiers, typedText, wheelMessage, windowLabel } from "./streamed-protocol";

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


describe("windows and notices", () => {
    const windowsMsg = {
        t: "windows",
        active: 1,
        windows: [
            { id: 0, url: "https://app.example/", title: "App", opener: null, closable: false },
            { id: 1, url: "https://idp.example/auth", title: "Sign in", opener: 0, closable: true },
        ],
    };

    it("reads the windows of the pane and which one is shown", () => {
        expect(parseServerMessage(JSON.stringify(windowsMsg))).toEqual(windowsMsg);
    });

    it("refuses a windows message that is not well formed", () => {
        expect(parseServerMessage(JSON.stringify({ ...windowsMsg, active: "1" }))).toBeNull();
        expect(parseServerMessage(JSON.stringify({ ...windowsMsg, windows: "none" }))).toBeNull();
        expect(parseServerMessage(JSON.stringify({ ...windowsMsg, windows: [{ url: "x" }] }))).toBeNull();
        expect(parseServerMessage(JSON.stringify({ ...windowsMsg, windows: [null] }))).toBeNull();
    });

    it("reads a notice as the words the server gave", () => {
        expect(parseServerMessage('{"t":"notice","message":"a.example opened a message box. It was dismissed."}')).toEqual({
            t: "notice",
            message: "a.example opened a message box. It was dismissed.",
        });
    });

    it("names a window by its site, so the person sees who is asking", () => {
        expect(windowLabel({ url: "https://login.example.com/a?b=1", title: "Sign in" })).toBe("login.example.com");
        expect(windowLabel({ url: "http://127.0.0.1:8080/x", title: "" })).toBe("127.0.0.1:8080");
        expect(windowLabel({ url: "about:blank", title: "Untitled" })).toBe("Untitled");
        expect(windowLabel({ url: "", title: "" })).toBe("…");
    });

    it("cuts a long host from the left, so the end that says whose site it is stays visible", () => {
        const long = "accounts.google.com.sign-in.verify-your-identity.evil-example.net";
        const label = windowLabel({ url: `https://${long}/login`, title: "Sign in" });
        expect(label.length).toBeLessThanOrEqual(LABEL_MAX);
        expect(label.startsWith("…")).toBe(true);
        expect(label.endsWith("evil-example.net")).toBe(true);
        expect(long.endsWith(label.slice(1))).toBe(true);
        // A host that fits is shown whole.
        expect(windowLabel({ url: "https://accounts.google.com/", title: "" })).toBe("accounts.google.com");
    });

    it("shows a title in a tab with nothing hidden in it, cut at its end", () => {
        expect(windowLabel({ url: "about:blank", title: "Pay\u202Epal\u200B now" })).toBe("Paypal now");
        const cut = windowLabel({ url: "about:blank", title: "x".repeat(100) });
        expect(cut.length).toBeLessThanOrEqual(LABEL_MAX);
        expect(cut.endsWith("…")).toBe(true);
    });

    it("shows what a page wrote in a notice with control, reordering and zero-width characters removed", () => {
        const text = "a.example opened a message box (\u201CSign\u202Ein\u200B to\u2066 pay\u0007\uFEFF\u201D). It was dismissed.";
        expect(parseServerMessage(JSON.stringify({ t: "notice", message: text }))).toEqual({
            t: "notice",
            message: "a.example opened a message box (“Signin to pay”). It was dismissed.",
        });
        expect(plainText("one\n  two\tthree")).toBe("one two three");
    });
});

describe("copying out of the remote browser", () => {
    it("reads the text the server brought over, cut at the limit, and nothing without text", () => {
        expect(parseServerMessage(JSON.stringify({ t: "clipboard", text: "Hello\nW\u00f6rld" }))).toEqual({ t: "clipboard", text: "Hello\nW\u00f6rld" });
        const long = parseServerMessage(JSON.stringify({ t: "clipboard", text: "x".repeat(CLIPBOARD_MAX + 5) }));
        expect(long?.t === "clipboard" && long.text.length).toBe(CLIPBOARD_MAX);
        expect(parseServerMessage(JSON.stringify({ t: "clipboard" }))).toBeNull();
        expect(parseServerMessage(JSON.stringify({ t: "clipboard", text: 5 }))).toBeNull();
    });

    it("on a Mac, sends Cmd with C, X, A, Z, F, L or R as Ctrl, and nothing else", () => {
        const cmd = { ...none, metaKey: true };
        for (const k of ["c", "X", "a", "z", "f", "l", "r"]) expect(remoteModifiers({ ...cmd, key: k }, true)).toMatchObject({ ctrlKey: true, metaKey: false });
        // Shift stays (redo), and the key message carries the Ctrl bit, not Meta.
        expect(keyMessage("down", { ...cmd, shiftKey: true, key: "z", code: "KeyZ", keyCode: 90 }, true)).toMatchObject({ mods: 2 | 8, text: "" });
        // Not on another platform; not for a letter the remote browser takes differently (T, W, Q); not with Ctrl or Alt held.
        expect(remoteModifiers({ ...cmd, key: "c" }, false).metaKey).toBe(true);
        for (const k of ["t", "w", "q"]) expect(remoteModifiers({ ...cmd, key: k }, true).metaKey).toBe(true);
        expect(remoteModifiers({ ...cmd, ctrlKey: true, key: "c" }, true).metaKey).toBe(true);
        expect(remoteModifiers({ ...cmd, altKey: true, key: "c" }, true).metaKey).toBe(true);
    });

    it("takes Ctrl with C or X as a copy, and nothing else", () => {
        expect(isCopyChord({ ...none, key: "c", ctrlKey: true })).toBe(true);
        expect(isCopyChord({ ...none, key: "X", ctrlKey: true })).toBe(true);
        expect(isCopyChord({ ...none, key: "c" })).toBe(false);
        expect(isCopyChord({ ...none, key: "v", ctrlKey: true })).toBe(false);
        expect(isCopyChord({ ...none, key: "c", ctrlKey: true, altKey: true })).toBe(false);
        expect(isCopyChord({ ...none, key: "c", metaKey: true })).toBe(false);
    });
});
