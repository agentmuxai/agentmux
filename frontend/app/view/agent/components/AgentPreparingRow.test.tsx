// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentNode } from "../types";
import { AgentPreparingRow } from "./AgentPreparingRow";

afterEach(cleanup);

const NOW = 5_000_000;
const start = (state: string, extra: Record<string, unknown> = {}): DocumentNode =>
    ({ type: "cli_notice", id: "s", kind: "container_start", provider: "claude", startState: state, timestamp: NOW, ...extra }) as unknown as DocumentNode;

describe("AgentPreparingRow", () => {
    it("shows Preparing… with the phase while a container agent starts, and clears when the start is over", () => {
        const [nodes, setNodes] = createSignal<readonly DocumentNode[]>([start("pulling", { percent: 10 })]);
        const { container } = render(() => <AgentPreparingRow nodes={nodes} enabled={() => true} now={() => NOW} />);
        const row = container.querySelector("[data-testid=agent-preparing-row]");
        expect(row?.textContent).toContain("Preparing…");
        expect(row?.textContent).toContain("Downloading the agent image… 10%");
        setNodes([start("ready", { seconds: 5 })]);
        expect(container.querySelector("[data-testid=agent-preparing-row]")).toBeNull();
    });

    it("shows nothing for a pane that is not a container agent", () => {
        const { container } = render(() => <AgentPreparingRow nodes={() => [start("starting")]} enabled={() => false} now={() => NOW} />);
        expect(container.querySelector("[data-testid=agent-preparing-row]")).toBeNull();
    });
});
