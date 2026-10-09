// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { makeTestHostApi } from "@/app/host/test-host";
import { onSrvInfo, UI_VERSION } from "@/app/store/srv-info";
import { VersionSkewStatus } from "./VersionSkewStatus";

afterEach(() => {
    cleanup();
    window.api = undefined as unknown as AppApi;
});

describe("VersionSkewStatus", () => {
    it("shows nothing when the backend is this UI's version", () => {
        onSrvInfo({ version: UI_VERSION });
        render(() => <VersionSkewStatus />);
        expect(screen.queryByText(/Reload for/)).toBeNull();
    });

    it("offers a reload when the backend is another version", () => {
        onSrvInfo({ version: "9.9.9" });
        render(() => <VersionSkewStatus />);
        expect(screen.getByText("Reload for 9.9.9")).toBeTruthy();
    });

    it("asks the host to reload the window when clicked", () => {
        const reloadWindow = vi.fn();
        window.api = makeTestHostApi({ reloadWindow });
        onSrvInfo({ version: "9.9.9" });
        render(() => <VersionSkewStatus />);
        fireEvent.click(screen.getByText("Reload for 9.9.9"));
        expect(reloadWindow).toHaveBeenCalledTimes(1);
    });
});
