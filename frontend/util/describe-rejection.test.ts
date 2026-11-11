// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import { describeEvent, describeRejection, isEvent } from "./describe-rejection";

function eventOn(target: object | null, type = "error"): Event {
    const ev = new Event(type);
    Object.defineProperty(ev, "target", { value: target });
    return ev;
}

describe("describeEvent", () => {
    it("names the event, the element and where it loads from", () => {
        const img = document.createElement("img");
        img.src = "https://example.com/favicon.ico";
        expect(describeEvent(eventOn(img))).toBe("error event on img https://example.com/favicon.ico");
        const link = document.createElement("link");
        link.href = "https://example.com/site.css";
        expect(describeEvent(eventOn(link))).toBe("error event on link https://example.com/site.css");
    });

    it("names a target that is not an element by its kind, with its url when it has one", () => {
        class FakeSocket {
            url = "wss://example.com/ws";
        }
        expect(describeEvent(eventOn(new FakeSocket(), "close"))).toBe("close event on FakeSocket wss://example.com/ws");
        expect(describeEvent(eventOn({}))).toBe("error event on Object");
        expect(describeEvent(eventOn(null, "abort"))).toBe("abort event");
    });

    it("shows a data URL by its kind and cuts a long address", () => {
        const img = document.createElement("img");
        img.src = `data:image/png;base64,${"A".repeat(5000)}`;
        expect(describeEvent(eventOn(img))).toBe("error event on img data:image/png;base64,…");
        img.src = `https://example.com/${"p".repeat(400)}`;
        const said = describeEvent(eventOn(img));
        expect(said.length).toBeLessThan(200);
        expect(said.endsWith("…")).toBe(true);
    });
});

describe("describeRejection", () => {
    it("says an Error's message, an event's description, and anything else as text", () => {
        expect(describeRejection(new Error("Failed to fetch"))).toBe("Failed to fetch");
        expect(describeRejection(new TypeError(""))).toBe("TypeError");
        expect(describeRejection(eventOn(null))).toBe("error event");
        expect(describeRejection("nope")).toBe("nope");
        expect(describeRejection({ code: 7 })).toBe('{"code":7}');
        expect(describeRejection(undefined)).toBe("undefined");
    });

    it("knows an event from what merely looks like one", () => {
        expect(isEvent(new Event("error"))).toBe(true);
        expect(isEvent({ isTrusted: true })).toBe(false);
    });
});
