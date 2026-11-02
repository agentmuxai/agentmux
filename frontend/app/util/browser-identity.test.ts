// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    newIncognitoIdentity,
    newTabUrl,
    parseIdentity,
} from "./browser-identity";

describe("browser tab identity (SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09 §5)", () => {
    it("reads an identity, and anything else as Personal", () => {
        expect(parseIdentity(undefined)).toEqual({ kind: "personal" });
        expect(parseIdentity("")).toEqual({ kind: "personal" });
        expect(parseIdentity("incognito:0f0e2d1c-aaaa")).toEqual({ kind: "incognito", jar: "0f0e2d1c-aaaa" });
        expect(parseIdentity("incognito:short")).toEqual({ kind: "personal" });
        expect(parseIdentity("profile:work")).toEqual({ kind: "profile", id: "work" });
        expect(parseIdentity(5)).toEqual({ kind: "personal" });
    });

    it("gives every new Incognito tab its own jar", () => {
        const a = newIncognitoIdentity();
        const b = newIncognitoIdentity();
        expect(a).not.toEqual(b);
        expect(parseIdentity(a).kind).toBe("incognito");
    });

    it("opens a new tab at the current page, or Home when it isn't a web page", () => {
        expect(newTabUrl("https://github.com/x", "https://home.example")).toBe("https://github.com/x");
        expect(newTabUrl("about:blank", "https://home.example")).toBe("https://home.example");
        expect(newTabUrl(undefined, "https://home.example")).toBe("https://home.example");
    });
});
