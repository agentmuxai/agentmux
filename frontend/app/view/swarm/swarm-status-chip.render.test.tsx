// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm status chip shows two states: working (red, mid-turn) and idle
 * (green, everything else). User request, 2026-09-27.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { AgentStatusChip, chipStatus, type AgentDisplayStatus } from "./swarm-view";

afterEach(() => cleanup());

describe("chipStatus", () => {
    it("mid-turn states are working", () => {
        for (const s of ["working", "tools", "stopping"] as AgentDisplayStatus[]) {
            expect(chipStatus(s)).toBe("working");
        }
    });

    it("everything else is idle", () => {
        for (const s of ["idle", "error", "disconnected", "unknown", "interrupted"] as AgentDisplayStatus[]) {
            expect(chipStatus(s)).toBe("idle");
        }
    });
});

describe("AgentStatusChip", () => {
    it("labels and classes the chip with one of the two states; 'tools' never appears", () => {
        const { container } = render(() => <AgentStatusChip status="tools" />);
        const chip = container.querySelector(".swarm-status-chip")!;
        expect(chip.textContent).toBe("working");
        expect(chip.classList.contains("swarm-status-chip--working")).toBe(true);
        expect(chip.querySelector(".swarm-status-dot--working")).not.toBeNull();
    });

    it("follows the status as it changes (props read reactively, not destructured once)", () => {
        const [status, setStatus] = createSignal<AgentDisplayStatus>("working");
        const { container } = render(() => <AgentStatusChip status={status()} />);
        const chip = () => container.querySelector(".swarm-status-chip")!;
        expect(chip().textContent).toBe("working");
        setStatus("idle");
        expect(chip().textContent).toBe("idle");
        expect(chip().classList.contains("swarm-status-chip--idle")).toBe(true);
        setStatus("tools");
        expect(chip().textContent).toBe("working");
    });
});
