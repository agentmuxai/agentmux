// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { installMediaMenuGuard, isMediaTarget } from "./media-menu-guard";

let remove: (() => void) | null = null;

afterEach(() => {
    remove?.();
    remove = null;
    document.body.innerHTML = "";
});

/** Fire a right-click on `el` and say whether the page left the engine's own menu to open. */
function rightClick(el: Element): { engineMenuOpens: boolean; reachedBody: boolean } {
    let reachedBody = false;
    const onBody = (): void => {
        reachedBody = true;
    };
    document.body.addEventListener("contextmenu", onBody);
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, composed: true, button: 2 });
    el.dispatchEvent(event);
    document.body.removeEventListener("contextmenu", onBody);
    return { engineMenuOpens: !event.defaultPrevented, reachedBody };
}

describe("the media menu guard", () => {
    it.each(["video", "audio"] as const)("cancels the engine's menu for a right-click on a %s", (tag) => {
        remove = installMediaMenuGuard();
        const media = document.body.appendChild(document.createElement(tag));
        media.setAttribute("controls", "");
        expect(rightClick(media).engineMenuOpens).toBe(false);
    });

    it("counts a right-click on something inside a media element's own tree", () => {
        // jsdom cannot give a <video> a shadow root, so the path an engine reports for its
        // controls (the target is inside the element) is stood in for.
        const video = document.createElement("video");
        const controls = document.createElement("div");
        const inside = { target: controls, composedPath: () => [controls, video, document.body] } as unknown as Event;
        expect(isMediaTarget(inside)).toBe(true);
        const outside = { target: controls, composedPath: () => [controls, document.body] } as unknown as Event;
        expect(isMediaTarget(outside)).toBe(false);
    });

    it("leaves the event going to the handlers that build the page's own menu", () => {
        remove = installMediaMenuGuard();
        const video = document.body.appendChild(document.createElement("video"));
        expect(rightClick(video).reachedBody).toBe(true);
    });

    it("leaves every other element's right-click alone", () => {
        remove = installMediaMenuGuard();
        const img = document.body.appendChild(document.createElement("img"));
        const div = document.body.appendChild(document.createElement("div"));
        expect(rightClick(img).engineMenuOpens).toBe(true);
        expect(rightClick(div).engineMenuOpens).toBe(true);
    });

    it("does nothing once removed", () => {
        const off = installMediaMenuGuard();
        off();
        const video = document.body.appendChild(document.createElement("video"));
        expect(rightClick(video).engineMenuOpens).toBe(true);
    });

    it("tells a media element from anything else", () => {
        const video = document.createElement("video");
        document.body.appendChild(video);
        const hit = new MouseEvent("contextmenu", { bubbles: true, composed: true });
        video.dispatchEvent(hit);
        expect(isMediaTarget(hit)).toBe(true);
        const div = document.body.appendChild(document.createElement("div"));
        const miss = new MouseEvent("contextmenu", { bubbles: true, composed: true });
        div.dispatchEvent(miss);
        expect(isMediaTarget(miss)).toBe(false);
    });
});
