// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A widget's pane whose widget isn't running says why, and offers Settings
 *  (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.4). */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";

const split = vi.fn(async () => "b2");
vi.mock("@/app/store/block-layout-actions", () => ({ createBlockSplitHorizontally: (...a: unknown[]) => split(...(a as [])) }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { WidgetsListCommand: async () => ({ packages: [] }) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
const loadErrors: Record<string, string> = {};
vi.mock("./widget-loader", () => ({ widgetLoadErrors: () => loadErrors }));

import { setWidgetPackages } from "@/app/store/widget-packages-store";
import { MissingWidget } from "./missing-widget";

const VIEW = "ext:acme.notes/main";

function pkg(over: Partial<WidgetPackageInfo> = {}): WidgetPackageInfo {
    return {
        id: "acme.notes",
        name: "Notes",
        version: "1.0.0",
        description: null,
        author: null,
        homepage: null,
        icon: "note-sticky",
        default_hue: null,
        kind: "sandboxed",
        permissions: [],
        granted: [],
        state: "approved",
        error: null,
        hash: "h",
        panes: [{ view: VIEW, name: "main", label: "Notes", icon: "note-sticky", entry: "index.html", singleton: false, default_meta: {} }],
        commands: [],
        status_items: [],
        signature: { state: "unsigned", publisher: "acme", fingerprint: null, pinned: null },
        files_url: null,
        implied: false,
        folder: "",
        ...over,
    };
}

afterEach(() => {
    cleanup();
    split.mockClear();
    delete loadErrors["acme.notes"];
});

describe("MissingWidget", () => {
    it("says the widget isn't installed", () => {
        setWidgetPackages([]);
        render(() => <MissingWidget blockId="b1" view={VIEW} />);
        expect(screen.getByRole("status").textContent).toContain("This widget isn't installed.");
    });

    it("says it waits for approval, and opens Settings → Widgets", () => {
        setWidgetPackages([pkg({ state: "needs_approval" })]);
        render(() => <MissingWidget blockId="b1" view={VIEW} />);
        expect(screen.getByRole("status").textContent).toContain("Notes is waiting for your approval.");
        fireEvent.click(screen.getByRole("button", { name: /Open Settings/ }));
        expect(split).toHaveBeenCalledWith({ meta: { view: "settings", "settings:section": "widgets" } }, "b1", "after");
    });

    it("names the other reasons", () => {
        for (const [over, text] of [
            [{ state: "changed" }, "Notes changed since you approved it"],
            [{ state: "disabled" }, "Notes is turned off."],
            // An invalid package lists no panes; it's found by the id in the view.
            [{ state: "invalid", error: "`version` isn't semver", panes: [] }, "Notes can't be loaded: `version` isn't semver."],
        ] as [Partial<WidgetPackageInfo>, string][]) {
            setWidgetPackages([pkg(over)]);
            render(() => <MissingWidget blockId="b1" view={VIEW} />);
            expect(screen.getByRole("status").textContent).toContain(text);
            cleanup();
        }
    });

    it("shows an approved widget's load error, or that it's loading", () => {
        setWidgetPackages([pkg()]);
        render(() => <MissingWidget blockId="b1" view={VIEW} />);
        expect(screen.getByRole("status").textContent).toContain("Loading Notes…");
        expect(screen.queryByRole("button")).toBeNull();
        cleanup();
        loadErrors["acme.notes"] = "its module threw";
        render(() => <MissingWidget blockId="b1" view={VIEW} />);
        expect(screen.getByRole("status").textContent).toContain("Notes didn't load: its module threw");
    });
});
