// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The prompt's user-facing strings. These are a security surface, not cosmetics:
 * the user's decision rests entirely on correctly understanding *who* is asking
 * and *for what*, so under-reporting either is the failure mode that matters.
 */

import { describe, expect, it } from "vitest";
import { describeRequestedDevices, displayOrigin } from "./pane-media-permission-prompt";

const AUDIO = 1 << 0;
const VIDEO = 1 << 1;
const DESKTOP_AUDIO = 1 << 2;
const DESKTOP_VIDEO = 1 << 3;

describe("describeRequestedDevices", () => {
    it("names a single device", () => {
        expect(describeRequestedDevices(VIDEO)).toBe("camera");
        expect(describeRequestedDevices(AUDIO)).toBe("microphone");
    });

    it("names a combined request with both devices", () => {
        // The case CEF's exact-match rule makes common: {audio, video} is one
        // indivisible request, so the prompt must say both.
        expect(describeRequestedDevices(AUDIO | VIDEO)).toBe("camera and microphone");
    });

    it("names desktop capture distinctly from device capture", () => {
        expect(describeRequestedDevices(DESKTOP_VIDEO)).toBe("screen contents");
        expect(describeRequestedDevices(DESKTOP_AUDIO)).toBe("system audio");
    });

    it("lists three or more with commas and a trailing 'and'", () => {
        expect(describeRequestedDevices(AUDIO | VIDEO | DESKTOP_VIDEO)).toBe(
            "camera, microphone and screen contents",
        );
    });

    it("surfaces unknown bits instead of silently dropping them", () => {
        // If CEF adds a permission bit we don't know, the prompt must not
        // under-report what the page asked for — the user would be agreeing to
        // something invisible.
        const unknown = 1 << 9;
        expect(describeRequestedDevices(VIDEO | unknown)).toBe("camera and other media devices");
        expect(describeRequestedDevices(unknown)).toBe("other media devices");
    });

    it("never returns an empty description", () => {
        expect(describeRequestedDevices(0)).toBe("media devices");
    });
});

describe("displayOrigin", () => {
    it("shows the host, not the full URL", () => {
        expect(displayOrigin("https://example.com")).toBe("example.com");
        expect(displayOrigin("https://sub.example.com:8443")).toBe("sub.example.com:8443");
    });

    it("does not let a long path push the real origin out of view", () => {
        // Spoofing shape: a path crafted to read like a different site.
        expect(displayOrigin("https://evil.test/https://bank.example.com/login")).toBe("evil.test");
    });

    it("falls back to the raw string rather than showing nothing", () => {
        expect(displayOrigin("not a url")).toBe("not a url");
    });

    it("has a non-empty fallback for an empty origin", () => {
        expect(displayOrigin("")).toBe("This page");
    });
});
