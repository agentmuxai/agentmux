// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Connectors and Memory panes (section-pane.tsx and the two manifests),
 * and the Armory and Remotes manifests that move saved blocks onto them.
 * docs/specs/SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md,
 * docs/specs/SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/view/accounts/accounts-manager", () => ({
    AccountsManager: () => <div data-testid="accounts-manager" />,
}));
vi.mock("@/app/view/global-bundle/global-bundle-manager", () => ({
    GlobalBundleManager: () => <div data-testid="global-bundle-manager" />,
}));
vi.mock("@/app/view/native-memory/native-memory-manager", () => ({
    NativeMemoryManager: () => <div data-testid="native-memory-manager" />,
}));
vi.mock("@/app/view/bundle/bundle-manager", () => ({
    BundleManager: () => <div data-testid="bundle-manager" />,
}));
vi.mock("@/app/view/mcp/mcp-manager", () => ({
    McpManager: () => <div data-testid="mcp-manager" />,
}));
vi.mock("@/app/view/skill/skill-manager", () => ({
    SkillManager: () => <div data-testid="skill-manager" />,
}));
// The Remotes list, standing in: it shows what the section gave its model.
const remotesModels = vi.hoisted(() => [] as { blockId: string; expandRequest: () => string; disposed: boolean }[]);
vi.mock("@/app/view/remotes/remotes-model", () => ({
    RemotesViewModel: class {
        blockId: string;
        expandRequest: () => string;
        disposed = false;
        constructor(ctx: { blockId: string; meta?: () => Record<string, unknown> | undefined }) {
            this.blockId = ctx.blockId;
            this.expandRequest = () => (ctx.meta?.()?.["remotes:expand"] as string | undefined) ?? "";
            remotesModels.push(this);
        }
        dispose() {
            this.disposed = true;
        }
    },
}));
vi.mock("@/app/view/remotes/remotes-view", () => ({
    RemotesView: (props: { model: { blockId: string; expandRequest: () => string } }) => (
        <div data-testid="remotes-view" data-block={props.model.blockId} data-expand={props.model.expandRequest()} />
    ),
}));
vi.mock("@/app/store/block-component-registry", () => ({
    openOrFocusPaneByView: vi.fn(() => Promise.resolve()),
}));

import type { PaneTabHostContext, PaneTabManifest } from "@/app/block/pane-tab-registry";
import { armoryPaneTab } from "@/app/view/armory/armory";
import { connectorsPaneTab } from "@/app/view/connectors/connectors";
import { memoryPaneTab } from "@/app/view/memory/memory";
import { remotesPaneTab } from "@/app/view/remotes/remotes";

// A real signal behind the host context, so a setMeta write flows back into
// the pane the way the backend's meta push does.
const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
const setMetaMock = vi.fn((patch: Record<string, unknown>) => {
    setBlockMeta((prev) => ({ ...prev, ...patch }));
    return Promise.resolve();
});

function makeCtx(): PaneTabHostContext {
    return {
        blockId: "test-block",
        meta: blockMeta as unknown as PaneTabHostContext["meta"],
        setMeta: setMetaMock,
        isFocused: () => false,
        visibility: () => "active",
    };
}

function mount(manifest: PaneTabManifest, meta: Record<string, unknown> = {}) {
    setBlockMeta(meta);
    const instance = createRoot(() => manifest.create(makeCtx()));
    const ctx = makeCtx();
    const result = render(() => instance.component({ ctx }));
    return { ...result, instance, title: () => instance.liveTitle!().text };
}

function tabLabels(container: HTMLElement): string[] {
    return Array.from(container.querySelectorAll('[role="tab"] .ui-tab-label')).map((s) => s.textContent ?? "");
}

function visiblePane(container: HTMLElement): string | null {
    const pane = container.querySelector(".bundle-manager-section > .bundle-manager-pane:not(.is-hidden)");
    return pane?.querySelector("[data-testid]")?.getAttribute("data-testid") ?? null;
}

afterEach(() => {
    cleanup();
    setMetaMock.mockClear();
    setBlockMeta({});
    remotesModels.length = 0;
});

describe("Connectors pane", () => {
    it("has the sections Accounts, MCP servers and Remotes, on Accounts by default", () => {
        const { container, title } = mount(connectorsPaneTab);
        expect(tabLabels(container)).toEqual(["Accounts", "MCP servers", "Remotes"]);
        expect(visiblePane(container)).toBe("accounts-manager");
        expect(title()).toBe("Connectors · Accounts");
    });

    it("opens on the section in connectors:section", () => {
        const { container, title } = mount(connectorsPaneTab, { "connectors:section": "mcp" });
        expect(visiblePane(container)).toBe("mcp-manager");
        expect(title()).toBe("Connectors · MCP servers");
    });

    it("falls back to Accounts for an unknown section", () => {
        const { container } = mount(connectorsPaneTab, { "connectors:section": "skills" });
        expect(visiblePane(container)).toBe("accounts-manager");
    });

    it("clicking a section writes connectors:section and shows it", () => {
        const { container, title } = mount(connectorsPaneTab);
        fireEvent.click(screen.getByRole("tab", { name: "MCP servers" }));
        expect(setMetaMock).toHaveBeenCalledWith({ "connectors:section": "mcp" });
        expect(visiblePane(container)).toBe("mcp-manager");
        expect(title()).toBe("Connectors · MCP servers");
    });
});

describe("Connectors → Remotes", () => {
    it("shows the Remotes list on this pane's block", () => {
        const { container, title } = mount(connectorsPaneTab, { "connectors:section": "remotes" });
        expect(visiblePane(container)).toBe("remotes-view");
        expect(title()).toBe("Connectors · Remotes");
        expect(container.querySelector('[data-testid="remotes-view"]')!.getAttribute("data-block")).toBe("test-block");
    });

    it("passes the pane's meta on, so another pane's link reaches the list", () => {
        const { container } = mount(connectorsPaneTab, { "connectors:section": "remotes" });
        setBlockMeta((m) => ({ ...m, "remotes:expand": "db1" }));
        expect(container.querySelector('[data-testid="remotes-view"]')!.getAttribute("data-expand")).toBe("db1");
    });

    it("disposes the list's model with the pane", () => {
        const { unmount } = mount(connectorsPaneTab);
        expect(remotesModels).toHaveLength(1);
        unmount();
        expect(remotesModels[0].disposed).toBe(true);
    });
});

describe("a saved Remotes block", () => {
    it("rewrites its meta to Connectors → Remotes after it's created, keeping the rest", async () => {
        setBlockMeta({ view: "remotes", "remotes:expand": "db1", "term:zoom": 1.3 });
        createRoot(() => remotesPaneTab.create(makeCtx()));
        expect(setMetaMock).not.toHaveBeenCalled();
        await Promise.resolve();
        expect(setMetaMock).toHaveBeenCalledWith({ view: "connectors", "connectors:section": "remotes" });
    });

    it("shows Connectors on Remotes before the write lands, and lets the user pick another section", () => {
        const { container, title } = mount(remotesPaneTab, { view: "remotes", "remotes:expand": "db1" });
        expect(visiblePane(container)).toBe("remotes-view");
        expect(title()).toBe("Connectors · Remotes");
        expect(container.querySelector('[data-testid="remotes-view"]')!.getAttribute("data-expand")).toBe("db1");
        fireEvent.click(screen.getByRole("tab", { name: "Accounts" }));
        expect(visiblePane(container)).toBe("accounts-manager");
    });

    it("stands in for Connectors", () => {
        expect(remotesPaneTab.legacyOf).toBe("connectors");
    });
});

describe("Memory pane", () => {
    it("has the sections Global, Personal, Skills and Bundles, on Global by default", () => {
        const { container, title } = mount(memoryPaneTab);
        expect(tabLabels(container)).toEqual(["Global", "Personal", "Skills", "Bundles"]);
        expect(visiblePane(container)).toBe("global-bundle-manager");
        expect(title()).toBe("Memory · Global");
    });

    it.each([
        ["personal", "native-memory-manager"],
        ["skills", "skill-manager"],
        ["bundles", "bundle-manager"],
    ])("opens on memory:section=%s", (section, testId) => {
        const { container } = mount(memoryPaneTab, { "memory:section": section });
        expect(visiblePane(container)).toBe(testId);
    });

    it("keeps every section mounted, hiding all but the selected one", () => {
        const { container } = mount(memoryPaneTab);
        expect(container.querySelectorAll(".bundle-manager-section > .bundle-manager-pane")).toHaveLength(4);
        expect(container.querySelectorAll(".bundle-manager-section > .bundle-manager-pane.is-hidden")).toHaveLength(3);
    });

    it("clicking a tab writes memory:section", () => {
        const { container } = mount(memoryPaneTab);
        fireEvent.click(screen.getByRole("tab", { name: "Skills" }));
        // The section key from when the pane was named Knowledge is cleared too.
        expect(setMetaMock).toHaveBeenCalledWith({ "knowledge:section": null, "memory:section": "skills" });
        expect(visiblePane(container)).toBe("skill-manager");
    });

    it("opens on the section a block saved when the pane was named Knowledge", () => {
        const { container } = mount(memoryPaneTab, { "knowledge:section": "bundles" });
        expect(visiblePane(container)).toBe("bundle-manager");
    });

    it("prefers memory:section over the old key when a block has both", () => {
        const { container } = mount(memoryPaneTab, { "memory:section": "skills", "knowledge:section": "bundles" });
        expect(visiblePane(container)).toBe("skill-manager");
    });

    // One tablist (element/ui TabbedPane) that is a rail or top tabs by width,
    // instead of a rail plus a hidden tab bar. It still precedes the sections,
    // so at narrow widths it sits at the top of the pane
    // (SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md).
    it("renders one tablist, before the sections", () => {
        const { container } = mount(memoryPaneTab);
        expect(screen.getAllByRole("tablist")).toHaveLength(1);
        const tablist = screen.getByRole("tablist", { name: "Memory section" });
        const panel = container.querySelector(".bundle-manager-section")!;
        expect(panel.getAttribute("role")).toBe("tabpanel");
        expect(tablist.compareDocumentPosition(panel) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it("highlights only Bundles", () => {
        const { container } = mount(memoryPaneTab);
        const highlighted = Array.from(container.querySelectorAll(".is-abf-highlight")).map((el) => el.textContent);
        expect(highlighted).toEqual(["Bundles"]);
    });
});

describe("section pane zoom", () => {
    const wheel = (el: Element, init: WheelEventInit) =>
        el.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, ...init }));

    it("applies term:zoom as CSS zoom", () => {
        const { container } = mount(connectorsPaneTab, { "term:zoom": 1.2 });
        expect((container.querySelector(".armory-view") as HTMLElement).style.zoom).toBe("1.2");
    });

    it("Ctrl+Wheel steps term:zoom; back at 1.0 it clears the key", () => {
        const { container } = mount(memoryPaneTab);
        const view = container.querySelector(".armory-view")!;
        wheel(view, { ctrlKey: true, deltaY: 100 });
        expect(setMetaMock).toHaveBeenLastCalledWith({ "term:zoom": 0.9 });
        wheel(view, { ctrlKey: true, deltaY: -100 });
        expect(setMetaMock).toHaveBeenLastCalledWith({ "term:zoom": null });
    });

    it("leaves a plain wheel and Ctrl+Shift+Wheel (all panes) alone", () => {
        const { container } = mount(memoryPaneTab);
        const view = container.querySelector(".armory-view")!;
        wheel(view, { deltaY: 100 });
        wheel(view, { ctrlKey: true, shiftKey: true, deltaY: 100 });
        expect(setMetaMock).not.toHaveBeenCalled();
    });
});

describe("a saved Armory block", () => {
    it("rewrites its meta to the new pane after it's created", async () => {
        setBlockMeta({ view: "armory", "armory:section": "skills", "term:zoom": 1.3 });
        createRoot(() => armoryPaneTab.create(makeCtx()));
        expect(setMetaMock).not.toHaveBeenCalled();
        await Promise.resolve();
        expect(setMetaMock).toHaveBeenCalledWith({
            view: "memory",
            "memory:section": "skills",
            "armory:section": null,
            "armory:memory:subsection": null,
        });
    });

    it("shows its new pane until the block remounts as it", () => {
        const { container, title } = mount(armoryPaneTab, { view: "trust", "armory:section": "mcp" });
        expect(tabLabels(container)).toEqual(["Accounts", "MCP servers", "Remotes"]);
        expect(title()).toBe("Connectors · Accounts");
        // The meta write lands: now the section follows.
        setBlockMeta((m) => ({ ...m, view: "connectors", "connectors:section": "mcp" }));
        expect(visiblePane(container)).toBe("mcp-manager");
        expect(title()).toBe("Connectors · MCP servers");
    });

    it("keeps the old Trust Center view id as an alias", () => {
        expect(armoryPaneTab.aliases).toEqual(["trust"]);
    });
});
