// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Warden rail (`WardenView`) — mirrors section-pane.test.tsx's
 * structure/pattern exactly, adapted for Warden's 5 sections (Host, LAN,
 * Internet, Audit, History).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setPlatform } from "@/util/platformutil";

vi.mock("@/app/view/warden-host/warden-host-manager", () => ({
    WardenHostManager: (props: { visible?: boolean }) => <div data-testid="host-manager" data-visible={String(props.visible)} />,
}));
vi.mock("@/app/view/warden-lan/warden-lan-manager", () => ({
    WardenLanManager: (props: { visible?: boolean }) => <div data-testid="lan-manager" data-visible={String(props.visible)} />,
}));
vi.mock("@/app/view/warden-internet/warden-internet-stub", () => ({
    WardenInternetStub: () => <div data-testid="internet-stub" />,
}));
vi.mock("@/app/view/warden-audit/warden-audit-manager", () => ({
    WardenAuditManager: (props: { visible?: boolean }) => <div data-testid="audit-manager" data-visible={String(props.visible)} />,
}));
vi.mock("@/app/view/warden-kanban/warden-kanban-manager", () => ({
    WardenKanbanManager: (props: { visible: boolean; onNeedsCount?: (n: number) => void }) => {
        props.onNeedsCount?.(2);
        return <div data-testid="kanban-manager" data-visible={String(props.visible)} />;
    },
}));
vi.mock("@/app/view/vault/vault-view", () => ({
    VaultView: (props: { model: { shown: () => boolean; zoomAtom: () => number } }) => (
        <div data-testid="vault-view" data-shown={String(props.model.shown())} data-zoom={String(props.model.zoomAtom())} />
    ),
}));
const ambientFailing = vi.hoisted(() => ({ n: 3 }));
vi.mock("@/app/view/warden-ambient/warden-ambient-manager", () => ({
    WardenAmbientManager: (props: { visible: boolean; onFailingCount?: (n: number) => void }) => {
        props.onFailingCount?.(ambientFailing.n);
        return <div data-testid="ambient-manager" data-visible={String(props.visible)} />;
    },
}));
vi.mock("@/app/view/warden-history/warden-history-manager", () => ({
    WardenHistoryManager: (props: { wardenBlockId: string; visible: boolean }) => (
        <div data-testid="history-manager" data-block={props.wardenBlockId} data-visible={String(props.visible)} />
    ),
}));

// blockMeta is a real Solid signal, not a plain object — warden-model.ts's
// sectionAtom/viewName are createMemo-derived from model.blockAtom(), so
// reading a plain (non-reactive) stub only ever satisfies a memo's *first*
// (eager, at-construction) computation. Backing the mock with a genuine
// signal, and having the host context's setMeta write into it, reproduces the
// real write -> MPS push -> blockAtom update round trip closely enough for
// clicking a rail item to actually flip the visible/active section here,
// the same way it does against the real backend. Mirrors section-pane.test.tsx.
const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});

const setMetaMock = vi.fn((patch: Record<string, unknown>) => {
    setBlockMeta((prev) => ({ ...prev, ...patch }));
    return Promise.resolve();
});

import { create as createWarden } from "./warden-instance";
import { WardenViewModel } from "./warden-model";
import { WardenView } from "./warden-view";

describe("WardenView rail", () => {
    afterEach(() => {
        cleanup();
        setBlockMeta({});
    });

    function renderWarden() {
        const model = new WardenViewModel({ blockId: "test-block", meta: blockMeta as any, setMeta: setMetaMock, isFocused: () => false, visibility: () => "active" });
        return render(() => (
            <WardenView
                model={model}
            />
        ));
    }

    it("orders the rail as Kanban, Vault, Host, LAN, Internet, Audit, History, Ambient", () => {
        renderWarden();
        const rail = screen.getByRole("tablist", { name: "Warden section" });
        const labels = Array.from(rail.querySelectorAll('[role="tab"]')).map((el) => el.textContent);
        expect(labels).toEqual(["Kanban", "Vault", "Host", "LAN", "Internet", "Audit", "History", "Ambient"]);
    });

    // The Vault became a section of Warden (vault-in-warden.ts): its protections follow the
    // section, not the pane, because Warden keeps its sections mounted.
    it("mounts the Vault only once its section is shown, and shows it only while selected", () => {
        renderWarden();
        expect(screen.queryByTestId("vault-view")).toBeNull();
        fireEvent.click(screen.getByRole("tab", { name: "Vault" }));
        expect(screen.getByTestId("vault-view").dataset.shown).toBe("true");
        fireEvent.click(screen.getByRole("tab", { name: "Host" }));
        // Still mounted (no second decrypt), but not shown: no on-screen note, Reveal closed.
        expect(screen.getByTestId("vault-view").dataset.shown).toBe("false");
    });

    it("gives the Vault no zoom of its own: Warden zooms the whole pane", () => {
        setBlockMeta({ "warden:section": "vault", "term:zoom": 1.5 });
        renderWarden();
        expect(screen.getByTestId("vault-view").dataset.zoom).toBe("1");
    });

    it("opens a block saved as the Vault pane on the Vault section, titled with the Vault's own", () => {
        setBlockMeta({ view: "vault", "vault:section": "cards" });
        const model = new WardenViewModel({ blockId: "b", meta: blockMeta as any, setMeta: setMetaMock, isFocused: () => false, visibility: () => "active" });
        expect(model.sectionAtom()).toBe("vault");
        expect(model.viewName()).toBe("Vault · Cards and banks");
        setBlockMeta({ view: "vault", "warden:section": "host" });
        expect(model.sectionAtom()).toBe("host");
    });

    it("shows the Vault's import, export and note actions only on the Vault section", () => {
        setBlockMeta({ "warden:section": "host" });
        const instance = createRoot(() =>
            createWarden({ blockId: "b", meta: blockMeta as any, setMeta: setMetaMock, isFocused: () => false, visibility: () => "active" })
        );
        expect(instance.headerActions?.()).toEqual([]);
        setBlockMeta({ "warden:section": "vault" });
        expect(instance.headerActions?.().map((a) => (a as { title?: string }).title)).toEqual([
            "Import into the Vault",
            "Export the Vault",
            "What the Vault may ask you",
        ]);
    });

    it("defaults to the Host section active and visible", () => {
        const { container } = renderWarden();
        expect(screen.getByTestId("host-manager")).toBeInTheDocument();
        const hostPane = screen.getByTestId("host-manager").closest(".bundle-manager-pane");
        expect(hostPane?.classList.contains("is-hidden")).toBe(false);
        // Not shown yet, so not mounted: its poll hasn't started.
        expect(screen.queryByTestId("audit-manager")).toBeNull();
        void container;
    });

    it("clicking a rail item switches the active/visible pane without unmounting others", () => {
        renderWarden();
        screen.getByRole("tab", { name: "Audit" }).click();

        const auditPane = screen.getByTestId("audit-manager").closest(".bundle-manager-pane");
        expect(auditPane?.classList.contains("is-hidden")).toBe(false);
        const hostPane = screen.getByTestId("host-manager").closest(".bundle-manager-pane");
        expect(hostPane?.classList.contains("is-hidden")).toBe(true);
        // Host manager is still mounted (in the DOM), just hidden — the
        // keep-everything-mounted pattern section-pane.tsx also uses.
        expect(screen.getByTestId("host-manager")).toBeInTheDocument();
    });

    it("mounts each section the first time it is shown, then keeps it mounted", () => {
        renderWarden();
        // Opened on Host: only Host is mounted, so no other section's poll runs.
        expect(screen.getByTestId("host-manager")).toBeInTheDocument();
        for (const id of ["kanban-manager", "lan-manager", "internet-stub", "audit-manager", "history-manager", "vault-view"]) {
            expect(screen.queryByTestId(id), id).toBeNull();
        }
        for (const name of ["Kanban", "LAN", "Internet", "Audit", "History"]) screen.getByRole("tab", { name }).click();
        for (const id of ["kanban-manager", "host-manager", "lan-manager", "internet-stub", "audit-manager", "history-manager"]) {
            expect(screen.getByTestId(id), id).toBeInTheDocument();
        }
    });

    it("tells Host, LAN and Audit when they are shown, so they poll only then", () => {
        renderWarden();
        screen.getByRole("tab", { name: "LAN" }).click();
        screen.getByRole("tab", { name: "Audit" }).click();
        expect(screen.getByTestId("host-manager").dataset.visible).toBe("false");
        expect(screen.getByTestId("lan-manager").dataset.visible).toBe("false");
        expect(screen.getByTestId("audit-manager").dataset.visible).toBe("true");
        screen.getByRole("tab", { name: "Host" }).click();
        expect(screen.getByTestId("host-manager").dataset.visible).toBe("true");
        expect(screen.getByTestId("audit-manager").dataset.visible).toBe("false");
    });

    it("Kanban knows when it is shown, so it only fetches then", () => {
        renderWarden();
        screen.getByRole("tab", { name: "Kanban" }).click();
        const kanban = screen.getByTestId("kanban-manager");
        expect(kanban.dataset.visible).toBe("true");
        screen.getByRole("tab", { name: "Host" }).click();
        expect(kanban.dataset.visible).toBe("false");
    });

    it("History gets the Warden block and knows when it is shown", () => {
        renderWarden();
        screen.getByRole("tab", { name: "History" }).click();
        const history = screen.getByTestId("history-manager");
        expect(history.dataset.visible).toBe("true");
        expect(history.dataset.block).toBeTruthy();
        expect(history.closest(".bundle-manager-pane")?.classList.contains("is-hidden")).toBe(false);
    });
});

describe("WardenView pane title", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
        setBlockMeta({});
    });

    function renderWarden() {
        const model = new WardenViewModel({ blockId: "test-block", meta: blockMeta as any, setMeta: setMetaMock, isFocused: () => false, visibility: () => "active" });
        const result = render(() => (
            <WardenView
                model={model}
            />
        ));
        return { ...result, model };
    }

    it("defaults viewName() to 'Host' with no warden:section meta", () => {
        const { model } = renderWarden();
        expect(model.viewName()).toBe("Host");
    });

    it("clicking a rail item writes warden:section through the host context and updates viewName()", () => {
        const { model } = renderWarden();
        screen.getByRole("tab", { name: "Audit" }).click();
        expect(setMetaMock).toHaveBeenCalledWith({ "warden:section": "audit" });
        expect(model.viewName()).toBe("Audit");
        const auditPane = screen.getByTestId("audit-manager").closest(".bundle-manager-pane");
        expect(auditPane?.classList.contains("is-hidden")).toBe(false);
    });

    it("on the Kanban section, the title counts the tasks that need the operator", () => {
        const { model } = renderWarden();
        screen.getByRole("tab", { name: "Kanban" }).click();
        expect(model.viewName()).toBe("Kanban · 2 need you");
    });

    it("selecting a section from the keyboard writes warden:section and updates viewName()", () => {
        const { model } = renderWarden();
        fireEvent.keyDown(screen.getByRole("tab", { name: "Host" }), { key: "End" });
        expect(setMetaMock).toHaveBeenCalledWith({ "warden:section": "ambient" });
        expect(model.viewName()).toBe("Ambient · 3 failing");
    });

    it("on the Ambient section, the title counts the failing kinds of call, and is plain with none", () => {
        const { model } = renderWarden();
        screen.getByRole("tab", { name: "Ambient" }).click();
        expect(screen.getByTestId("ambient-manager").dataset.visible).toBe("true");
        expect(model.viewName()).toBe("Ambient · 3 failing");
        cleanup();
        setBlockMeta({});
        ambientFailing.n = 0;
        const second = renderWarden();
        screen.getByRole("tab", { name: "Ambient" }).click();
        expect(second.model.viewName()).toBe("Ambient");
        ambientFailing.n = 3;
    });

    // SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md
    // One tablist that is a rail or top tabs by width; it precedes the content,
    // so at narrow widths it sits at the top of the pane.
    it("renders one tablist, before the content section", () => {
        renderWarden();
        expect(screen.getAllByRole("tablist")).toHaveLength(1);
        const tablist = screen.getByRole("tablist", { name: "Warden section" });
        const section = document.querySelector(".bundle-manager-section");
        expect(tablist.compareDocumentPosition(section as Node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it("viewName() reflects a pre-seeded warden:section meta value", () => {
        setBlockMeta({ "warden:section": "lan" });
        const model = new WardenViewModel({ blockId: "test-block", meta: blockMeta as any, setMeta: setMetaMock, isFocused: () => false, visibility: () => "active" });
        expect(model.viewName()).toBe("LAN");
    });

    it("falls back to 'Host' for an invalid warden:section meta value", () => {
        setBlockMeta({ "warden:section": "not-a-real-section" });
        const model = new WardenViewModel({ blockId: "test-block", meta: blockMeta as any, setMeta: setMetaMock, isFocused: () => false, visibility: () => "active" });
        expect(model.viewName()).toBe("Host");
    });
});

describe("WardenView zoom", () => {
    // mod+wheel: Ctrl on Windows/Linux (these tests), ⌘ on macOS.
    beforeEach(() => setPlatform("win32"));
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
        setBlockMeta({});
    });

    function renderWarden() {
        const model = new WardenViewModel({ blockId: "test-block", meta: blockMeta as any, setMeta: setMetaMock, isFocused: () => false, visibility: () => "active" });
        const result = render(() => (
            <WardenView
                model={model}
            />
        ));
        return { ...result, model };
    }

    it("applies model.zoomAtom() as a CSS zoom style on the root", () => {
        const { container } = renderWarden();
        const view = container.querySelector(".warden-view") as HTMLElement;
        expect(view.style.zoom).toBe("1");
    });

    // No zoom handler of its own: Ctrl+wheel is not stopped here, so it reaches the
    // window's shared handler (AppZoomHandler, app.tsx), which steps every pane the
    // same way and shows the indicator. SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md §3.
    it("leaves Ctrl+Wheel to the window's shared zoom handler", () => {
        setPlatform("win32");
        const { container } = renderWarden();
        const view = container.querySelector(".warden-view")!;
        const seen = vi.fn();
        window.addEventListener("wheel", seen);
        const ev = new WheelEvent("wheel", { ctrlKey: true, deltaY: 100, bubbles: true, cancelable: true });
        view.dispatchEvent(ev);
        window.removeEventListener("wheel", seen);
        expect(seen).toHaveBeenCalledTimes(1);
        expect(ev.defaultPrevented).toBe(false);
        expect(setMetaMock).not.toHaveBeenCalled();
    });

    it("plain wheel (no Ctrl) does not trigger a zoom RPC call", () => {
        const { container } = renderWarden();
        const view = container.querySelector(".warden-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: false, deltaY: 100, bubbles: true, cancelable: true }));
        expect(setMetaMock).not.toHaveBeenCalled();
    });

    // Ctrl+Shift+Scroll is AppAllPanesZoomHandler's all-panes gesture
    // (app.tsx) — this pane's own handler must let it through rather than
    // also zooming itself, or the two handlers would double-step this pane
    // relative to every other pane in the window. See
    // docs/specs/SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md.
    it("Ctrl+Shift+Wheel does not trigger this pane's own zoom RPC call", () => {
        const { container } = renderWarden();
        const view = container.querySelector(".warden-view") as HTMLElement;
        view.dispatchEvent(
            new WheelEvent("wheel", { ctrlKey: true, shiftKey: true, deltaY: 100, bubbles: true, cancelable: true })
        );
        expect(setMetaMock).not.toHaveBeenCalled();
    });

});
