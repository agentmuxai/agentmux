// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { readSigninOffer, signinOfferQuestion } from "./browser-signin-offer-model";

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
