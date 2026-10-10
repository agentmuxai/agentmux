// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A widget's palette commands and status bar items
 *  (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.8). */

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";

const commands = new Map<string, { label: string; category: string; icon?: string; execute: () => unknown }>();
vi.mock("@/app/store/command-registry", () => ({
    commandRegistry: {
        register: (e: { id: string; label: string; category: string; execute: () => unknown }) => {
            commands.set(e.id, e);
            return () => commands.delete(e.id);
        },
    },
}));

import { statusBarItems } from "@/app/statusbar/status-bar-registry";
import { registerWidgetContributions, runWidgetCommand, type ContributionDeps } from "./widget-contributions";
import { __resetWidgetPanes, setStatusLook, trackWidgetPane } from "./widget-panes";

function pkg(): WidgetPackageInfo {
    return {
        id: "acme.prs",
        name: "PR dashboard",
        version: "1.0.0",
        description: null,
        author: null,
        homepage: null,
        icon: "code-pull-request",
        default_hue: null,
        kind: "sandboxed",
        permissions: [],
        granted: [],
        state: "approved",
        error: null,
        hash: "h",
        panes: [{ view: "ext:acme.prs/main", name: "main", label: "PRs", icon: "code-pull-request", entry: "index.html", singleton: false, default_meta: { "widget:acme.prs:repo": "a/b" } }],
        commands: [{ id: "refresh", title: "Refresh", icon: "rotate-right", view: "ext:acme.prs/main", keywords: "reload" }],
        status_items: [{ id: "count", text: "PRs", icon: "code-pull-request", tooltip: "Open pull requests", command: "refresh", alignment: "right" }],
        files_url: "/x/",
        implied: false,
        folder: "",
    };
}

function deps(over: Partial<ContributionDeps> = {}): ContributionDeps & { opened: BlockDef[] } {
    const opened: BlockDef[] = [];
    return {
        opened,
        focusedBlockId: () => null,
        focusPane: () => true,
        openPane: async (def) => {
            opened.push(def);
            return "new";
        },
        ...over,
    };
}

afterEach(() => {
    __resetWidgetPanes();
    document.body.innerHTML = "";
});

describe("palette commands", () => {
    it("are listed under the widget's name, and leave with the package", () => {
        const off = registerWidgetContributions(pkg(), deps());
        const cmd = commands.get("ext:acme.prs/refresh");
        expect(cmd).toMatchObject({ label: "PR dashboard: Refresh", category: "Widgets", icon: "rotate-right" });
        expect(statusBarItems("right").map((i) => i.id)).toContain("widget:acme.prs/count");
        for (const u of off) u();
        expect(commands.has("ext:acme.prs/refresh")).toBe(false);
        expect(statusBarItems("right").map((i) => i.id)).not.toContain("widget:acme.prs/count");
    });

    it("run in the focused pane of the widget first, else another in this tab", async () => {
        const a = vi.fn();
        const b = vi.fn();
        trackWidgetPane("a", { pkgId: "acme.prs", view: "ext:acme.prs/main", command: a });
        trackWidgetPane("b", { pkgId: "acme.prs", view: "ext:acme.prs/main", command: b });
        const focused: string[] = [];
        const d = deps({ focusedBlockId: () => "b", focusPane: (id) => (focused.push(id), true) });
        expect(await runWidgetCommand(pkg(), "ext:acme.prs/main", "refresh", "palette", d)).toBe(true);
        expect(b).toHaveBeenCalledWith("refresh", "palette");
        expect(a).not.toHaveBeenCalled();
        expect(focused).toEqual(["b"]);
        expect(d.opened).toEqual([]);
    });

    it("open a pane when none is in this tab, and reach it once it loads", async () => {
        trackWidgetPane("elsewhere", { pkgId: "acme.prs", view: "ext:acme.prs/main", command: vi.fn() });
        const d = deps({ focusPane: () => false });
        const running = runWidgetCommand(pkg(), "ext:acme.prs/main", "refresh", "status", d);
        await Promise.resolve();
        expect(d.opened).toEqual([{ meta: { "widget:acme.prs:repo": "a/b", view: "ext:acme.prs/main" } }]);
        const fresh = vi.fn();
        trackWidgetPane("new", { pkgId: "acme.prs", view: "ext:acme.prs/main", command: fresh });
        expect(await running).toBe(true);
        expect(fresh).toHaveBeenCalledWith("refresh", "status");
    });
});

describe("status items", () => {
    function mount() {
        registerWidgetContributions(pkg(), deps());
        const item = statusBarItems("right").find((i) => i.id === "widget:acme.prs/count")!;
        const host = document.createElement("div");
        document.body.appendChild(host);
        render(() => item.render(), host);
        return () => host.querySelector<HTMLElement>("[role=button]");
    }

    it("show the manifest's look, named by the widget, until a pane sets another", () => {
        const button = mount();
        expect(button()?.textContent).toBe("PRs");
        expect(button()?.dataset.tip).toBe("PR dashboard: Open pull requests");
        trackWidgetPane("p1", { pkgId: "acme.prs", view: "ext:acme.prs/main" });
        setStatusLook("acme.prs", "count", "p1", { text: "3 PRs", tone: "warning", tooltip: "3 open" });
        expect(button()?.textContent).toBe("3 PRs");
        expect(button()?.dataset.tip).toBe("PR dashboard: 3 open");
        expect(button()?.classList.contains("tone-warning")).toBe(true);
        setStatusLook("acme.prs", "count", "p1", { hidden: true });
        expect(button()).toBeNull();
    });

    it("go back to the manifest's look when the pane that set it closes", () => {
        const button = mount();
        const forget = trackWidgetPane("p1", { pkgId: "acme.prs", view: "ext:acme.prs/main" });
        setStatusLook("acme.prs", "count", "p1", { text: "3 PRs" });
        expect(button()?.textContent).toBe("3 PRs");
        forget();
        expect(button()?.textContent).toBe("PRs");
    });

    it("never take an icon that isn't a Font Awesome name", () => {
        const button = mount();
        trackWidgetPane("p1", { pkgId: "acme.prs", view: "ext:acme.prs/main" });
        setStatusLook("acme.prs", "count", "p1", { icon: 'x" onclick="alert(1)' });
        expect(button()?.querySelector("i")?.className).toBe("fa fa-solid fa-code-pull-request status-icon");
    });
});
