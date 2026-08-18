// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// reagentx P1 on PR #2647 — see BundleMcpSection.test.tsx's identical
// doc comment, mirrored here for skills.

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

vi.mock("@/app/store/wps", () => ({ waveEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SkillCatalogListForBundleCommand: vi.fn(),
        SkillCatalogBindToBundleCommand: vi.fn(),
        SkillCatalogUnbindFromBundleCommand: vi.fn(),
        SkillCatalogUpsertForBundleCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { BundleSkillsSection } from "./BundleSkillsSection";

afterEach(() => cleanup());

beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([]);
});

function getInputs() {
    return {
        name: screen.getByPlaceholderText("Skill name") as HTMLInputElement,
        content: screen.getByPlaceholderText(/Skill content/) as HTMLTextAreaElement,
        submit: screen.getByRole("button", { name: /Add private skill/ }) as HTMLButtonElement,
    };
}

describe("BundleSkillsSection — add-private form", () => {
    test("a failed add does NOT clear the typed name/content", async () => {
        vi.mocked(RpcApi.SkillCatalogUpsertForBundleCommand).mockRejectedValue(
            new Error("skill name 'X' already bound to this bundle"),
        );
        render(() => <BundleSkillsSection bundleId="bundle-1" />);
        const user = userEvent.setup();
        const { name, content, submit } = getInputs();

        await user.type(name, "My Skill");
        await user.type(content, "Do the thing");
        await user.click(submit);

        await screen.findByText(/Add failed/);
        expect(name.value).toBe("My Skill");
        expect(content.value).toBe("Do the thing");
    });

    test("a successful add DOES clear the form", async () => {
        vi.mocked(RpcApi.SkillCatalogUpsertForBundleCommand).mockResolvedValue({
            id: "new-1", name: "My Skill", trigger: "", skill_type: "prompt",
            description: "", content: "Do the thing", is_global: false, created_at: 0, updated_at: 0,
        } as any);
        render(() => <BundleSkillsSection bundleId="bundle-1" />);
        const user = userEvent.setup();
        const { name, content, submit } = getInputs();

        await user.type(name, "My Skill");
        await user.click(submit);

        await vi.waitFor(() => expect(name.value).toBe(""));
        expect(content.value).toBe("");
    });
});
