// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { onSrvInfo, UI_VERSION } from "@/app/store/srv-info";
import { VersionSkewStatus } from "./VersionSkewStatus";

afterEach(cleanup);

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
});
