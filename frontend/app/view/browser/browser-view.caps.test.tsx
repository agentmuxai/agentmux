// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A host without native browser panes (HostCaps.nativeBrowserPane false)
 * shows a notice in a browser pane and never asks the host to create one.
 * docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md §5, slice 3.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { makeTestHostApi } from "@/app/host/test-host";
import { BrowserViewComponent } from "./browser-view";

afterEach(() => cleanup());

describe("browser pane on a host without native browser panes", () => {
    it("shows a notice and never creates a pane", () => {
        const create = vi.fn();
        window.api = makeTestHostApi({ browserPanes: { create } as unknown as BrowserPaneHostApi });
        // The fallback never touches the model.
        render(() => <BrowserViewComponent model={{} as any} />);
        expect(screen.getByText("Browser panes need the AgentMux desktop app.")).toBeTruthy();
        expect(create).not.toHaveBeenCalled();
    });
});
