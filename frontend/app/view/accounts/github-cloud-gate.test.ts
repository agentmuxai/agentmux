// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { MuxBusDeliveryStatus } from "@/store/muxbus-delivery";
import { GITHUB_CLOUD_SIGN_IN_TEXT, githubNeedsCloudSignIn, isNeedsCloudSignInError, NEEDS_CLOUD_SIGN_IN } from "./github-cloud-gate";

const state = (s: MuxBusDeliveryStatus["state"]): MuxBusDeliveryStatus => ({ state: s, since_ms: 1 }) as MuxBusDeliveryStatus;

describe("the GitHub connect's Cloud sign-in gate", () => {
    it("says what the sign-in is for", () => {
        expect(GITHUB_CLOUD_SIGN_IN_TEXT).toBe("Sign in to AgentMux Cloud to connect GitHub");
    });

    it("asks for the sign-in when the Cloud is signed out or needs it", () => {
        expect(githubNeedsCloudSignIn("github", true, state("signed_out"))).toBe(true);
        expect(githubNeedsCloudSignIn("github", true, state("needs_sign_in"))).toBe(true);
    });

    it("lets a signed-in or reconnecting Cloud connect", () => {
        expect(githubNeedsCloudSignIn("github", true, state("connected"))).toBe(false);
        expect(githubNeedsCloudSignIn("github", true, state("reconnecting"))).toBe(false);
    });

    it("waits for srv's answer rather than flashing the prompt", () => {
        expect(githubNeedsCloudSignIn("github", true, null)).toBe(false);
        expect(githubNeedsCloudSignIn("github", true, undefined)).toBe(false);
    });

    it("leaves the person's own app and other providers alone", () => {
        expect(githubNeedsCloudSignIn("github", false, state("signed_out"))).toBe(false);
        expect(githubNeedsCloudSignIn("google", true, state("signed_out"))).toBe(false);
    });

    it("knows srv's code, and only that", () => {
        expect(isNeedsCloudSignInError(NEEDS_CLOUD_SIGN_IN)).toBe(true);
        expect(NEEDS_CLOUD_SIGN_IN).toBe("needs-cloud-sign-in");
        expect(isNeedsCloudSignInError("blocked by your organization")).toBe(false);
        expect(isNeedsCloudSignInError(null)).toBe(false);
    });
});
