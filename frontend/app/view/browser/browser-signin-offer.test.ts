// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { readSavedSignins, readSigninOffer, savedSigninLabel, signinOfferQuestion } from "./browser-signin-offer-model";

describe("the Save to Vault offer", () => {
    it("is read from browser:signin_offer, and nothing else is an offer", () => {
        const offer = { id: "o1", origin: "https://accounts.example.com", user: "al***", update: false };
        expect(readSigninOffer({ "browser:signin_offer": offer })).toEqual(offer);
        expect(readSigninOffer({ "browser:signin_offer": null })).toBeUndefined();
        expect(readSigninOffer({ "browser:signin_offer": { origin: "https://x.example" } })).toBeUndefined();
        expect(readSigninOffer(undefined)).toBeUndefined();
    });

    it("asks with only the origin and the masked user name srv gave", () => {
        expect(signinOfferQuestion({ id: "o1", origin: "https://accounts.example.com", user: "al***", update: false })).toBe(
            "Save the sign-in for https://accounts.example.com (al***) to the Vault?"
        );
        expect(signinOfferQuestion({ id: "o1", origin: "https://accounts.example.com", user: "", update: true })).toBe(
            "Update the saved password for https://accounts.example.com?"
        );
    });
});

describe("saved sign-ins for the key button", () => {
    it("reads only well-formed entries from the meta, never anything else", () => {
        const meta = {
            "browser:signins": [
                { entry: "e1", origin: "https://accounts.example.com", user: "al***", password: "x" },
                { entry: "e2", origin: "https://accounts.example.com" },
                { origin: "https://accounts.example.com", user: "sa***" },
                null,
            ],
        };
        expect(readSavedSignins(meta)).toEqual([
            { entry: "e1", origin: "https://accounts.example.com", user: "al***" },
            { entry: "e2", origin: "https://accounts.example.com", user: "" },
        ]);
        expect(readSavedSignins({ "browser:signins": "e1" })).toEqual([]);
        expect(readSavedSignins(undefined)).toEqual([]);
    });

    it("names the user, and the site only when the page has forms of two", () => {
        const a = { entry: "e1", origin: "https://accounts.example.com", user: "al***" };
        const b = { entry: "e2", origin: "https://login.example.net", user: "" };
        expect(savedSigninLabel(a, [a])).toBe("Sign in as al***");
        expect(savedSigninLabel(a, [a, b])).toBe("Sign in as al*** on https://accounts.example.com");
        expect(savedSigninLabel(b, [a, b])).toBe("Sign in (no user name) on https://login.example.net");
    });
});
