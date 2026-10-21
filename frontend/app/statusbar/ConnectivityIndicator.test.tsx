// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The status bar's Offline indicator: hidden while everything answers, named services when not. */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ConnectivityStatusCommand: vi.fn(async () => null) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import {
    acceptConnectivity,
    connectivity,
    connectivityCoversMuxbus,
    DEGRADED_QUIET_MS,
    type ServiceStatus,
} from "@/store/connectivity";
import { clockTime } from "@/store/muxbus-delivery";
import { ConnectivityIndicator } from "./ConnectivityIndicator";

let revision = 1;

function svc(service: string, label: string, state: ServiceStatus["state"], sinceMs: number): ServiceStatus {
    return { service, label, state, since_ms: sinceMs, last_error: state === "online" ? null : "connection refused" };
}

/** Sends srv's status with a newer revision than the last. */
function send(...services: ServiceStatus[]): void {
    revision += 1;
    acceptConnectivity({ revision, services });
}

const online = (at: number) => svc("internet", "Internet", "online", at);

describe("ConnectivityIndicator", () => {
    afterEach(() => cleanup());

    it("shows nothing while every service answers", () => {
        send(online(Date.now()), svc("model_api:anthropic", "Anthropic API", "online", Date.now()), svc("muxbus", "MuxBus", "online", Date.now()));
        const { container } = render(() => <ConnectivityIndicator />);
        expect(container.textContent).toBe("");
    });

    it("says Offline, red, naming each unreachable service and since when", () => {
        const since = Date.now() - 60_000;
        send(
            svc("internet", "Internet", "offline", since),
            svc("model_api:anthropic", "Anthropic API", "offline", since),
            svc("muxbus", "MuxBus", "offline", since)
        );
        render(() => <ConnectivityIndicator />);
        const item = screen.getByRole("status");
        expect(item.textContent).toContain("Offline");
        expect(item.classList.contains("status-connectivity--error")).toBe(true);
        const at = clockTime(since);
        expect(item.getAttribute("data-tip")).toBe(
            `No internet connection since ${at}. Can't reach the Anthropic API since ${at}; MuxBus since ${at}. ` +
                "AgentMux keeps checking and clears this when they answer."
        );
    });

    it("names only the service that is down when the internet is fine", () => {
        const since = Date.now() - 5_000;
        send(online(since), svc("model_api:anthropic", "Anthropic API", "offline", since), svc("muxbus", "MuxBus", "online", since));
        render(() => <ConnectivityIndicator />);
        const tip = screen.getByRole("status").getAttribute("data-tip") ?? "";
        expect(tip).toContain("Can't reach the Anthropic API since");
        expect(tip).not.toContain("No internet");
        expect(tip).not.toContain("MuxBus");
    });

    it("waits before showing a degraded service, then says so in amber", () => {
        send(online(Date.now()), svc("muxbus", "MuxBus", "degraded", Date.now() - 1_000));
        const { container } = render(() => <ConnectivityIndicator />);
        expect(container.textContent).toBe("");
        send(online(Date.now()), svc("muxbus", "MuxBus", "degraded", Date.now() - DEGRADED_QUIET_MS - 1_000));
        const item = screen.getByRole("status");
        expect(item.textContent).toContain("Connection issues");
        expect(item.classList.contains("status-connectivity--warn")).toBe(true);
        expect(item.getAttribute("data-tip")).toContain("Some requests to MuxBus since");
    });

    it("takes an API srv hasn't checked yet, and says nothing about it", () => {
        send(online(Date.now()), svc("model_api:anthropic", "Anthropic API", "unknown", Date.now()));
        expect(connectivity()?.services[1].state).toBe("unknown");
        render(() => <ConnectivityIndicator />);
        expect(screen.queryByText("Offline")).toBeNull();
    });

    it("ignores an older status that arrives late", () => {
        send(online(Date.now()), svc("muxbus", "MuxBus", "online", Date.now()));
        const current = connectivity()!.revision;
        acceptConnectivity({ revision: current - 1, services: [svc("muxbus", "MuxBus", "offline", Date.now())] });
        expect(connectivity()!.revision).toBe(current);
        // Malformed: a state srv doesn't send.
        acceptConnectivity({ revision: current + 1, services: [{ service: "muxbus", state: "gone" }] });
        expect(connectivity()!.revision).toBe(current);
    });

    it("tells the MuxBus indicator when it already names MuxBus", () => {
        const now = Date.now();
        send(online(now), svc("muxbus", "MuxBus", "offline", now));
        expect(connectivityCoversMuxbus(connectivity(), now)).toBe(true);
        send(online(now), svc("muxbus", "MuxBus", "online", now), svc("model_api:anthropic", "Anthropic API", "offline", now));
        expect(connectivityCoversMuxbus(connectivity(), now)).toBe(false);
    });
});
