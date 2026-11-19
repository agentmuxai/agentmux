// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** How a standing sign-in grant reads: the agent by its name where the pane knows it (a shared login's grant is
 *  stored under the agent's id), and the prompt it answers when it is one for an HTTP sign-in prompt. */

import { describe, expect, it } from "vitest";

import { grantText, signinGrantAgent, signinGrantLabel, type AgentNames } from "./vault-grant-labels";

const UID = "d96f0fae-d8b4-4816-a821-c8034c4f6ada";
const names: AgentNames = (agent) => (agent === UID ? "Tester" : undefined);

describe("a sign-in grant as a person reads it", () => {
    it("keeps the stored name of a grant that was given to an agent by name", () => {
        const grant = "signin:Agent1|personal|https://accounts.example.com";
        expect(signinGrantLabel(grant)).toBe("Agent1 on accounts.example.com");
        expect(signinGrantLabel(grant, names)).toBe("Agent1 on accounts.example.com");
        expect(signinGrantAgent(grant)).toBe("Agent1");
    });

    it("names the agent of a shared login's grant, which is stored under its id", () => {
        const grant = `signin:${UID}|profile:agent-tester-1|https://example.com|https://example.com`;
        expect(signinGrantAgent(grant)).toBe(UID);
        expect(signinGrantLabel(grant, names)).toBe("Tester on example.com");
        expect(grantText(grant, names)).toBe("signs in: Tester on example.com");
    });

    it("shows what was stored for an id it has no name for, and before the names are read", () => {
        const grant = `signin:${UID}|profile:agent-tester-1|https://example.com|https://example.com`;
        expect(signinGrantLabel(grant)).toBe(`${UID} on example.com`);
        expect(signinGrantLabel(grant, () => undefined)).toBe(`${UID} on example.com`);
    });

    it("says which sign-in prompt a grant answers, by its realm, a realm with a bar in it included", () => {
        const base = `signin:${UID}|profile:agent-tester-1|https://example.com|prompt:site|`;
        expect(signinGrantLabel(`${base}Staff area`, names)).toBe("Tester on example.com (sign-in prompt Staff area)");
        expect(signinGrantLabel(`${base}Staff|area`, names)).toBe("Tester on example.com (sign-in prompt Staff|area)");
        expect(signinGrantLabel(base, names)).toBe("Tester on example.com (sign-in prompt)");
    });

    it("says a grant is for a proxy's sign-in", () => {
        const grant = `signin:${UID}|profile:agent-tester-1|http://proxy.example.com:3128|prompt:proxy|`;
        expect(signinGrantLabel(grant, names)).toBe("Tester on the proxy proxy.example.com:3128");
    });

    it("has only the agent to show for a grant with no site", () => {
        expect(signinGrantLabel(`signin:${UID}`, names)).toBe("Tester");
    });
});
