// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

import { REMOTE_PROVIDER_UNKNOWN, SwarmProviderIcon } from "./swarm-provider-icon";

afterEach(cleanup);

// REPORT_SWARM_REMOTE_PROVIDER_ICON_RCA_2026_10_18.md §6.2.
describe("SwarmProviderIcon", () => {
    it("draws the provider's logo when one is known", () => {
        const { container } = render(() => <SwarmProviderIcon provider="claude" unknownTitle="x" />);
        expect(container.querySelector(".provider-logo")).not.toBeNull();
        expect(container.querySelector(".swarm-provider-unknown")).toBeNull();
    });

    it("draws a neutral mark, never the AgentMux logo, when none is known", () => {
        for (const provider of [undefined, null, "", "  "]) {
            const { container } = render(() => (
                <SwarmProviderIcon provider={provider} unknownTitle={REMOTE_PROVIDER_UNKNOWN} />
            ));
            const mark = container.querySelector(".swarm-provider-unknown")!;
            expect(mark).not.toBeNull();
            expect(mark.getAttribute("title")).toBe(REMOTE_PROVIDER_UNKNOWN);
            expect(container.querySelector(".provider-logo")).toBeNull();
            expect(container.querySelector("img")).toBeNull();
            cleanup();
        }
    });
});
