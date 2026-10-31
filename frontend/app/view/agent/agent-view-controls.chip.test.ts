// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// The composer chip says what the pane shows, and says nothing while it shows everything.

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/block-atom-cache", () => ({ getBlockMetaKeyAtom: vi.fn(), getSettingsKeyAtom: vi.fn() }));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { viewChipLabel } from "./agent-view-controls";
import { VIEW_PRESETS } from "./virtualization/verbosity";

describe("viewChipLabel", () => {
    it("is empty for a pane that follows a default of Full", () => {
        expect(viewChipLabel({ view: VIEW_PRESETS.full, source: "default" })).toBeNull();
    });

    it("names the preset a pane follows from the default", () => {
        expect(viewChipLabel({ view: VIEW_PRESETS.standard, source: "default" })).toBe("Standard");
    });

    it("adds this pane when the pane keeps its own view, even a plain Full", () => {
        expect(viewChipLabel({ view: VIEW_PRESETS.replies, source: "pane" })).toBe("Replies · this pane");
        expect(viewChipLabel({ view: VIEW_PRESETS.full, source: "pane" })).toBe("Full · this pane");
    });

    it("says Custom for a view that matches no preset", () => {
        expect(viewChipLabel({ view: { ...VIEW_PRESETS.standard, thinking: "hidden" }, source: "pane" })).toBe("Custom · this pane");
    });
});
