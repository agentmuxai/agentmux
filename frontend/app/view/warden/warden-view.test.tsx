// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Warden rail (`WardenView`) — mirrors armory-view.test.tsx's
 * structure/pattern exactly, adapted for Warden's 5 sections (Host, LAN,
 * Internet, Audit, Supervisor).
 */

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

const setMetaMock = vi.fn().mockResolvedValue(undefined);
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

describe("WardenView zoom", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
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
