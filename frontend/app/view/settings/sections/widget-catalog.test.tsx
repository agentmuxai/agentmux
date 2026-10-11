// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings → Widgets → Browse the catalog
 *  (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §4.5). */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { WidgetCatalogItem } from "@/app/store/rpc-api/widgets";

const catalog = vi.fn();
const install = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        WidgetsCatalogCommand: (...a: unknown[]) => catalog(...a),
        WidgetsCatalogInstallCommand: (...a: unknown[]) => install(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { __setCatalogItems, inCatalog, WidgetCatalog } from "./widget-catalog";

const FP = "K7Q2-MZ4D-PX3A-9TWE";

function item(over: Partial<WidgetCatalogItem> = {}): WidgetCatalogItem {
    return {
        entry: {
            id: "agentmux.notes",
            name: "Notes",
            version: "1.1.0",
            description: "A scratchpad",
            author: "AgentMux",
            homepage: null,
            icon: "note-sticky",
            permissions: ["storage"],
            hash: "h2",
            publisherKey: "k",
            zip: "https://agentmuxai.github.io/widgets/agentmux.notes-1.1.0.zip",
            zipSha256: "z",
        },
        fingerprint: FP,
        installed_version: null,
        installed_state: null,
        current: false,
        ...over,
    };
}

afterEach(() => {
    cleanup();
    __setCatalogItems([]);
    catalog.mockReset();
    install.mockReset();
});

describe("WidgetCatalog", () => {
    it("lists the catalog with who signed each widget and what it may do, and installs one", async () => {
        catalog.mockResolvedValue({ url: "u", items: [item(), item({ entry: { ...item().entry, id: "agentmux.hello", name: "Hello", permissions: [] }, current: true, installed_version: "1.0.0", installed_state: "approved" })] });
        install.mockResolvedValue({ id: "agentmux.notes", packages: [] });
        const onInstalled = vi.fn();
        render(() => <WidgetCatalog onInstalled={onInstalled} />);
        fireEvent.click(screen.getByText("Browse the catalog"));
        const notes = (await screen.findByText("Notes")).closest(".widget-row") as HTMLElement;
        expect(within(notes).getByText(`By AgentMux · signed by ${FP}`)).toBeInTheDocument();
        expect(within(notes).getByText("Keep its own data on this computer")).toBeInTheDocument();
        const hello = screen.getByText("Hello").closest(".widget-row") as HTMLElement;
        expect(within(hello).getByText("Installed")).toBeInTheDocument();
        expect(within(hello).queryByText("Install")).not.toBeInTheDocument();
        fireEvent.click(within(notes).getByText("Install"));
        await waitFor(() => expect(onInstalled).toHaveBeenCalledWith("agentmux.notes", []));
        expect(install.mock.calls[0][1]).toEqual({ id: "agentmux.notes" });
    });

    it("offers an update for an older installed version", async () => {
        catalog.mockResolvedValue({ url: "u", items: [item({ installed_version: "1.0.0", installed_state: "approved" })] });
        render(() => <WidgetCatalog onInstalled={() => {}} />);
        fireEvent.click(screen.getByText("Browse the catalog"));
        expect(await screen.findByText("Update to 1.1.0")).toBeInTheDocument();
    });

    it("says plainly when the catalog can't be read", async () => {
        catalog.mockRejectedValue(new Error("the catalog's signature doesn't verify"));
        render(() => <WidgetCatalog onInstalled={() => {}} />);
        fireEvent.click(screen.getByText("Browse the catalog"));
        expect(await screen.findByRole("alert")).toHaveTextContent("signature doesn't verify");
    });

    it("knows a package is the catalog's only at its files and its publisher's key", () => {
        __setCatalogItems([item()]);
        const sig = (fingerprint: string | null) => ({ state: "signed" as const, publisher: "agentmux", fingerprint, pinned: null });
        expect(inCatalog({ id: "agentmux.notes", hash: "h2", signature: sig(FP) })).toBe(true);
        expect(inCatalog({ id: "agentmux.notes", hash: "other", signature: sig(FP) })).toBe(false);
        expect(inCatalog({ id: "agentmux.notes", hash: "h2", signature: sig("AAAA-BBBB-CCCC-DDDD") })).toBe(false);
    });
});
