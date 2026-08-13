// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Warden rail (`WardenView`) — mirrors armory-view.test.tsx's
 * structure/pattern exactly, adapted for Warden's 5 sections (Host, LAN,
 * Internet, Audit, Supervisor).
 */

import { createSignal } from "solid-js";
import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/view/warden-host/warden-host-manager", () => ({
    WardenHostManager: () => <div data-testid="host-manager" />,
}));
vi.mock("@/app/view/warden-lan/warden-lan-manager", () => ({
    WardenLanManager: () => <div data-testid="lan-manager" />,
}));
vi.mock("@/app/view/warden-internet/warden-internet-stub", () => ({
    WardenInternetStub: () => <div data-testid="internet-stub" />,
}));
vi.mock("@/app/view/warden-audit/warden-audit-manager", () => ({
    WardenAuditManager: () => <div data-testid="audit-manager" />,
}));
vi.mock("@/app/view/warden-supervisor/warden-supervisor-manager", () => ({
    WardenSupervisorManager: () => <div data-testid="supervisor-manager" />,
}));

// blockMeta is a real Solid signal, not a plain object — warden-model.ts's
// sectionAtom/viewName are createMemo-derived from model.blockAtom(), so
// reading a plain (non-reactive) stub only ever satisfies a memo's *first*
// (eager, at-construction) computation. Backing the mock with a genuine
// signal, and having the SetMetaCommand mock write into it, reproduces the
// real write -> WPS push -> blockAtom update round trip closely enough for
// clicking a rail item to actually flip the visible/active section here,
// the same way it does against the real backend. Mirrors armory-view.test.tsx.
const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
vi.mock("@/app/store/wos", () => ({
    makeORef: (type: string, id: string) => `${type}:${id}`,
    getWaveObjectAtom: () => () => ({ meta: blockMeta() }),
    getObjectValue: () => ({}),
}));

const setMetaMock = vi.fn((..._args: unknown[]) => {
    const opts = _args[1] as { oref: string; meta: Record<string, unknown> };
    setBlockMeta((prev) => ({ ...prev, ...opts.meta }));
    return Promise.resolve(undefined);
});
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: (...args: unknown[]) => setMetaMock(...args),
    },
}));

import { WardenView } from "./warden-view";
import { WardenViewModel } from "./warden-model";

describe("WardenView rail", () => {
    afterEach(() => {
        cleanup();
        setBlockMeta({});
    });

    function renderWarden() {
        const model = new WardenViewModel("test-block", null as any);
        return render(() => (
            <WardenView
                blockId="test-block"
                model={model}
                blockRef={{ current: null }}
                contentRef={{ current: null }}
            />
        ));
    }

    it("orders the rail as Host, LAN, Internet, Audit, Supervisor", () => {
        renderWarden();
        const rail = screen.getByLabelText("Warden section", { selector: "nav.bundle-manager-rail" });
        const labels = Array.from(rail.querySelectorAll("button span")).map((el) => el.textContent);
        expect(labels).toEqual(["Host", "LAN", "Internet", "Audit", "Supervisor"]);
    });

    it("defaults to the Host section active and visible", () => {
        const { container } = renderWarden();
        expect(screen.getByTestId("host-manager")).toBeInTheDocument();
        const hostPane = screen.getByTestId("host-manager").closest(".bundle-manager-pane");
        expect(hostPane?.classList.contains("is-hidden")).toBe(false);
        const supervisorPane = screen.getByTestId("supervisor-manager").closest(".bundle-manager-pane");
        expect(supervisorPane?.classList.contains("is-hidden")).toBe(true);
        void container;
    });

    it("clicking a rail item switches the active/visible pane without unmounting others", () => {
        renderWarden();
        const rail = screen.getByLabelText("Warden section", { selector: "nav.bundle-manager-rail" });
        const auditButton = Array.from(rail.querySelectorAll("button")).find(
            (b) => b.textContent?.includes("Audit"),
        ) as HTMLButtonElement;
        auditButton.click();

        const auditPane = screen.getByTestId("audit-manager").closest(".bundle-manager-pane");
        expect(auditPane?.classList.contains("is-hidden")).toBe(false);
        const hostPane = screen.getByTestId("host-manager").closest(".bundle-manager-pane");
        expect(hostPane?.classList.contains("is-hidden")).toBe(true);
        // Host manager is still mounted (in the DOM), just hidden — the
        // keep-everything-mounted pattern armory-view.tsx also uses.
        expect(screen.getByTestId("host-manager")).toBeInTheDocument();
    });

    it("all 5 sections mount simultaneously (keep-mounted pane pattern)", () => {
        renderWarden();
        expect(screen.getByTestId("host-manager")).toBeInTheDocument();
        expect(screen.getByTestId("lan-manager")).toBeInTheDocument();
        expect(screen.getByTestId("internet-stub")).toBeInTheDocument();
        expect(screen.getByTestId("audit-manager")).toBeInTheDocument();
        expect(screen.getByTestId("supervisor-manager")).toBeInTheDocument();
    });
});

describe("WardenView pane title", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
        setBlockMeta({});
    });

    function renderWarden() {
        const model = new WardenViewModel("test-block", null as any);
        const result = render(() => (
            <WardenView
                blockId="test-block"
                model={model}
                blockRef={{ current: null }}
                contentRef={{ current: null }}
            />
        ));
        return { ...result, model };
    }

    it("defaults viewName() to 'Host' with no warden:section meta", () => {
        const { model } = renderWarden();
        expect(model.viewName()).toBe("Host");
    });

    it("clicking a rail item writes warden:section via SetMetaCommand and updates viewName()", () => {
        const { model } = renderWarden();
        const rail = screen.getByLabelText("Warden section", { selector: "nav.bundle-manager-rail" });
        const auditButton = Array.from(rail.querySelectorAll("button")).find(
            (b) => b.textContent?.includes("Audit"),
        ) as HTMLButtonElement;
        auditButton.click();
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "warden:section": "audit" } },
        );
        expect(model.viewName()).toBe("Audit");
        const auditPane = screen.getByTestId("audit-manager").closest(".bundle-manager-pane");
        expect(auditPane?.classList.contains("is-hidden")).toBe(false);
    });

    it("clicking a bottom tab-bar item writes warden:section via SetMetaCommand and updates viewName()", () => {
        const { model } = renderWarden();
        const tabBar = screen.getByLabelText("Warden section", { selector: "nav.bundle-manager-tab-bar" });
        const supervisorButton = Array.from(tabBar.querySelectorAll("button")).find(
            (b) => b.textContent?.includes("Supervisor"),
        ) as HTMLButtonElement;
        supervisorButton.click();
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "warden:section": "supervisor" } },
        );
        expect(model.viewName()).toBe("Supervisor");
    });

    it("viewName() reflects a pre-seeded warden:section meta value", () => {
        setBlockMeta({ "warden:section": "lan" });
        const model = new WardenViewModel("test-block", null as any);
        expect(model.viewName()).toBe("LAN");
    });

    it("falls back to 'Host' for an invalid warden:section meta value", () => {
        setBlockMeta({ "warden:section": "not-a-real-section" });
        const model = new WardenViewModel("test-block", null as any);
        expect(model.viewName()).toBe("Host");
    });
});

describe("WardenView zoom", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
        setBlockMeta({});
    });

    function renderWarden() {
        const model = new WardenViewModel("test-block", null as any);
        const result = render(() => (
            <WardenView
                blockId="test-block"
                model={model}
                blockRef={{ current: null }}
                contentRef={{ current: null }}
            />
        ));
        return { ...result, model };
    }

    it("applies model.zoomAtom() as a CSS zoom style on the root", () => {
        const { container } = renderWarden();
        const view = container.querySelector(".warden-view") as HTMLElement;
        expect(view.style.zoom).toBe("1");
    });

    it("Ctrl+Wheel down writes a decreased term:zoom via SetMetaCommand", () => {
        const { container } = renderWarden();
        const view = container.querySelector(".warden-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: 100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "term:zoom": 0.9 } },
        );
    });

    it("Ctrl+Wheel up writes an increased term:zoom via SetMetaCommand", () => {
        const { container } = renderWarden();
        const view = container.querySelector(".warden-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "term:zoom": 1.1 } },
        );
    });

    it("plain wheel (no Ctrl) does not trigger a zoom RPC call", () => {
        const { container } = renderWarden();
        const view = container.querySelector(".warden-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: false, deltaY: 100, bubbles: true, cancelable: true }));
        expect(setMetaMock).not.toHaveBeenCalled();
    });

    it("returning to 1.0 clears the metadata key (writes null)", () => {
        const model = new WardenViewModel("test-block", null as any);
        (model as any).zoomAtom = () => 0.9;
        const { container } = render(() => (
            <WardenView blockId="test-block" model={model} blockRef={{ current: null }} contentRef={{ current: null }} />
        ));
        const view = container.querySelector(".warden-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "term:zoom": null } },
        );
    });
});
