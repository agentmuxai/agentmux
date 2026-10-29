// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// GitHub through AgentMux's own App is an AgentMux Cloud feature: without a Cloud
// sign-in the account row says so and offers the sign-in, never a bare error
// (the Connect with GitHub design).

import type { MuxBusDeliveryStatus } from "@/store/muxbus-delivery";
import { muxbusOffersSignIn } from "@/store/muxbus-delivery";

/** What srv answers when a GitHub connect needs the sign-in (`oauth_client::NEEDS_CLOUD_SIGN_IN`). */
export const NEEDS_CLOUD_SIGN_IN = "needs-cloud-sign-in";

/** The sentence the account row shows in place of Connect. */
export const GITHUB_CLOUD_SIGN_IN_TEXT = "Sign in to AgentMux Cloud to connect GitHub";

/** Whether a flow's error is srv asking for the Cloud sign-in. */
export function isNeedsCloudSignInError(error: string | null | undefined): boolean {
    return error === NEEDS_CLOUD_SIGN_IN;
}

/**
 * Whether the GitHub connect shows the sign-in prompt instead of Connect: the connect uses
 * AgentMux's own App (not a client the person brought) and the Cloud isn't signed in. Unknown
 * until srv has said (`delivery` null), so the prompt doesn't flash before the state is in.
 */
export function githubNeedsCloudSignIn(
    provider: string,
    usesAgentMuxApp: boolean,
    delivery: MuxBusDeliveryStatus | null | undefined,
): boolean {
    return provider === "github" && usesAgentMuxApp && delivery != null && muxbusOffersSignIn(delivery);
}
