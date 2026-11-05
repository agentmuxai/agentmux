// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A bundle's skills as cards: the bound ones with Unbind or Remove and their content, the rest as chips to bind. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
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
import type { SkillBundleListItem } from "@/types/rpc/SkillBundleListItem";
import { BundleSkillsSection } from "./BundleSkillsSection";

const list = vi.mocked(RpcApi.SkillCatalogListForBundleCommand);

function skill(over: Partial<SkillBundleListItem> = {}): SkillBundleListItem {
    return {
        bound_to_bundle: true,
        id: "s1",
        name: "pdf-extraction",
        trigger: "",
        skill_type: "prompt",
        description: "Reads PDFs",
        content: "Extract the text.",
        is_global: true,
        created_at: 1,
        updated_at: 1,
        ...over,
    };
}

const grid = () => screen.getByLabelText("Skills in this bundle");
const card = (name: string) => within(grid()).getByText(name).closest(".ui-card") as HTMLElement;
const meta = (el: HTMLElement) => [...el.querySelectorAll(".ui-card-meta > *")].map((e) => e.textContent);

beforeEach(() => {
    vi.clearAllMocks();
    list.mockResolvedValue([]);
    vi.mocked(RpcApi.SkillCatalogBindToBundleCommand).mockResolvedValue(undefined as never);
    vi.mocked(RpcApi.SkillCatalogUnbindFromBundleCommand).mockResolvedValue(undefined as never);
});
afterEach(cleanup);

describe("the skills a bundle holds", () => {
    it("is a Skills category with a card for each bound skill", async () => {
        list.mockResolvedValue([
            skill({ id: "s1", name: "pdf-extraction" }),
            skill({ id: "s2", name: "private-one", is_global: false, description: "" }),
        ]);
        render(() => <BundleSkillsSection bundleId="b1" />);
        await screen.findByText("pdf-extraction");
        expect(screen.getByRole("heading", { level: 3, name: "Skills" })).toBeTruthy();
        expect(list).toHaveBeenCalledWith({}, { bundle_id: "b1" });
        expect(grid().querySelectorAll(".ui-card")).toHaveLength(2);
        expect(card("pdf-extraction").querySelector(".ui-card-subtitle")?.textContent).toBe("Reads PDFs");
        expect(card("private-one").querySelector(".ui-card-subtitle")).toBeNull();
        expect(meta(card("pdf-extraction"))).toEqual(["Global skill"]);
        expect(meta(card("private-one"))).toEqual(["Private to this bundle"]);
    });

    it("says so when it holds none", async () => {
        render(() => <BundleSkillsSection bundleId="b1" />);
        expect(await screen.findByText("No skills yet")).toBeTruthy();
        expect(screen.queryByLabelText("Skills in this bundle")).toBeNull();
    });

    it("unbinds a global skill and removes a private one", async () => {
        list.mockResolvedValue([
            skill({ id: "s1", name: "global-one" }),
            skill({ id: "s2", name: "private-one", is_global: false }),
        ]);
        render(() => <BundleSkillsSection bundleId="b1" />);
        await screen.findByText("global-one");
        fireEvent.click(within(card("global-one")).getByRole("button", { name: "Unbind" }));
        await waitFor(() =>
            expect(RpcApi.SkillCatalogUnbindFromBundleCommand).toHaveBeenCalledWith(
                {},
                { bundle_id: "b1", skill_id: "s1" }
            )
        );
        fireEvent.click(within(card("private-one")).getByRole("button", { name: "Remove" }));
        await waitFor(() =>
            expect(RpcApi.SkillCatalogUnbindFromBundleCommand).toHaveBeenCalledWith(
                {},
                { bundle_id: "b1", skill_id: "s2" }
            )
        );
    });

    it("does not open a skill's details when its Unbind is clicked", async () => {
        list.mockResolvedValue([skill()]);
        render(() => <BundleSkillsSection bundleId="b1" />);
        await screen.findByText("pdf-extraction");
        fireEvent.click(within(card("pdf-extraction")).getByRole("button", { name: "Unbind" }));
        expect(screen.queryByText("Extract the text.")).toBeNull();
    });

    it("shows a skill's content under it on a click, and closes it on the next", async () => {
        list.mockResolvedValue([skill(), skill({ id: "s2", name: "other", content: "Other content" })]);
        render(() => <BundleSkillsSection bundleId="b1" />);
        await screen.findByText("pdf-extraction");
        expect(screen.queryByText("Extract the text.")).toBeNull();
        fireEvent.click(card("pdf-extraction"));
        expect(await screen.findByText("Extract the text.")).toBeTruthy();
        expect(screen.getByText("A global skill: edit it in Memory > Skills.")).toBeTruthy();
        // Another card takes the details over.
        fireEvent.click(card("other"));
        expect(await screen.findByText("Other content")).toBeTruthy();
        expect(screen.queryByText("Extract the text.")).toBeNull();
        fireEvent.click(card("other"));
        await waitFor(() => expect(screen.queryByText("Other content")).toBeNull());
    });

    it("says a private skill's details belong to the bundle, and says when it has no content", async () => {
        list.mockResolvedValue([skill({ is_global: false, content: "" })]);
        render(() => <BundleSkillsSection bundleId="b1" />);
        fireEvent.click(await screen.findByText("pdf-extraction"));
        expect(await screen.findByText("(no content)")).toBeTruthy();
        expect(screen.getByText("A private skill belongs to this bundle: remove it to delete it.")).toBeTruthy();
    });

    it("offers each global skill not yet bound as a chip that binds it", async () => {
        list.mockResolvedValue([
            skill({ id: "s1", name: "bound-one" }),
            skill({ id: "s2", name: "free-global", bound_to_bundle: false }),
            skill({ id: "s3", name: "free-private", bound_to_bundle: false, is_global: false }),
        ]);
        render(() => <BundleSkillsSection bundleId="b1" />);
        await screen.findByText("bound-one");
        const chips = screen.getByLabelText("Global skills to bind");
        expect([...chips.querySelectorAll(".ui-card--chip")].map((c) => c.getAttribute("aria-label"))).toEqual([
            "Bind free-global",
        ]);
        fireEvent.click(screen.getByLabelText("Bind free-global"));
        await waitFor(() =>
            expect(RpcApi.SkillCatalogBindToBundleCommand).toHaveBeenCalledWith({}, { bundle_id: "b1", skill_id: "s2" })
        );
    });

    it("offers no chips when every global skill is bound", async () => {
        list.mockResolvedValue([skill()]);
        render(() => <BundleSkillsSection bundleId="b1" />);
        await screen.findByText("pdf-extraction");
        expect(screen.queryByLabelText("Global skills to bind")).toBeNull();
        expect(screen.queryByText("Bind a global skill:")).toBeNull();
    });

    it("still offers chips when the bundle holds none", async () => {
        list.mockResolvedValue([skill({ bound_to_bundle: false })]);
        render(() => <BundleSkillsSection bundleId="b1" />);
        expect(await screen.findByText("No skills yet")).toBeTruthy();
        expect(screen.getByLabelText("Bind pdf-extraction")).toBeTruthy();
        expect(screen.getByText("Bind a global skill:")).toBeTruthy();
    });

    it("shows why the skills could not be loaded", async () => {
        list.mockRejectedValue(new Error("boom"));
        render(() => <BundleSkillsSection bundleId="b1" />);
        expect(await screen.findByText("Failed to load skills: boom")).toBeTruthy();
    });

    it("keeps the private-skill form under the cards", async () => {
        render(() => <BundleSkillsSection bundleId="b1" />);
        await screen.findByText("No skills yet");
        expect(screen.getByPlaceholderText("Skill name")).toBeTruthy();
        expect(screen.getByRole("button", { name: /Add private skill/ })).toBeTruthy();
    });
});
