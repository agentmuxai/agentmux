// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one pane-request primitive
 * (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4.1,
 * Agent1's pane-modal spec, #18804, §3): requests are
 * kept per pane, shown in its dock or as its modal, never the window's; each
 * is a source of the pane's "waiting for you"; a closed pane gives the safe answer.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const started = vi.fn();
const ended = vi.fn();
vi.mock("@/app/notification/waiting-for-you", () => ({
    startWaitingForYou: (...a: unknown[]) => started(...a),
    endWaitingForYou: (...a: unknown[]) => ended(...a),
}));
// Panes that exist, by id: a test deletes one by setting it to undefined.
const panes = new Map<string, ReturnType<typeof createSignal<object | undefined>>>();
const pane = (id: string) => {
    if (!panes.has(id)) panes.set(id, createSignal<object | undefined>({ oid: id }));
    return panes.get(id)!;
};
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: (key: string) => pane(key.replace("block:", ""))[0] },
}));

import { useContext } from "solid-js";
import { OperatorOnlyModals } from "@/app/element/modal";
import { PaneRequestDock, PaneRequestModals } from "./PaneRequestDock";
import { __resetPaneRequests, openPaneModal, paneRequests, type PaneRequestProps } from "./pane-requests";

const Card = (p: { label: string } & PaneRequestProps) => (
    <div data-testid="card">
        {p.label}
        <button onClick={p.close}>Done</button>
    </div>
);

afterEach(() => {
    cleanup();
    __resetPaneRequests();
    started.mockClear();
    ended.mockClear();
    panes.clear();
});

describe("paneRequests", () => {
    it("keeps each pane's requests to itself, in the order they came", () => {
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "one" } });
        paneRequests.push("b", { source: "vault-use", Component: Card, props: { label: "other pane" } });
        paneRequests.push("a", { source: "vault-request", Component: Card, props: { label: "two" } });
        expect(paneRequests.of("a").map((r) => r.props.label)).toEqual(["one", "two"]);
        expect(paneRequests.of("b").map((r) => r.props.label)).toEqual(["other pane"]);
    });

    it("is one waiting source per request, ended when it is answered, once", () => {
        const h = paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "x" }, text: "Builder wants GitHub token" });
        expect(started).toHaveBeenCalledWith("a", expect.stringContaining("vault-use"), "Builder wants GitHub token");
        const key = started.mock.calls[0][1];
        h.close();
        h.close();
        expect(ended).toHaveBeenCalledTimes(1);
        expect(ended).toHaveBeenCalledWith("a", key, "submitted");
        expect(paneRequests.of("a")).toEqual([]);
    });

    it("starts no wait for a request nothing waits on", () => {
        paneRequests.push("a", { source: "info", Component: Card, props: { label: "x" }, waits: false });
        expect(started).not.toHaveBeenCalled();
    });

    it("gives the safe answer when its pane is closed, and only for that pane", async () => {
        const goneA = vi.fn();
        const goneB = vi.fn();
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "x" }, onGone: goneA });
        paneRequests.push("b", { source: "vault-use", Component: Card, props: { label: "y" }, onGone: goneB });
        pane("a")[1](undefined);
        await Promise.resolve();
        await Promise.resolve();
        expect(goneA).toHaveBeenCalledTimes(1);
        expect(goneB).not.toHaveBeenCalled();
        expect(paneRequests.of("a")).toEqual([]);
        expect(ended).toHaveBeenCalledWith("a", expect.any(String), "closed");
    });

    it("an answered request gives no safe answer later", async () => {
        const gone = vi.fn();
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "x" }, onGone: gone }).close();
        pane("a")[1](undefined);
        await Promise.resolve();
        await Promise.resolve();
        expect(gone).not.toHaveBeenCalled();
    });
});

describe("PaneRequestDock", () => {
    it("shows nothing when the pane has no card", () => {
        const r = render(() => <PaneRequestDock blockId="a" />);
        expect(r.queryByTestId("pane-request-dock")).toBeNull();
    });

    it("shows the first card in full and the rest as lines, operator-only, and a line shows its card", () => {
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "GitHub token" }, summary: "GitHub token" });
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "Stripe key" }, summary: "Stripe key (0:12)" });
        paneRequests.push("a", { source: "modal", Component: Card, props: { label: "a modal" }, presentation: "modal" });
        const r = render(() => <PaneRequestDock blockId="a" />);
        const dock = r.getByTestId("pane-request-dock");
        expect(dock.hasAttribute("data-operator-only")).toBe(true);
        expect(r.getAllByTestId("card").map((c) => c.textContent)).toEqual([expect.stringContaining("GitHub token")]);
        expect(r.getByTestId("pane-request-more").textContent).toContain("1 more:");
        fireEvent.click(r.getByText("Stripe key (0:12)"));
        expect(r.getAllByTestId("card").map((c) => c.textContent)).toEqual([expect.stringContaining("Stripe key")]);
        expect(r.getByTestId("pane-request-more").textContent).toContain("GitHub token");
    });

    it("never takes the focus from the composer", () => {
        const r = render(() => (
            <>
                <textarea data-testid="composer" />
                <PaneRequestDock blockId="a" />
            </>
        ));
        const composer = r.getByTestId("composer") as HTMLTextAreaElement;
        composer.focus();
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "x" } });
        expect(r.getByTestId("card")).toBeTruthy();
        expect(document.activeElement).toBe(composer);
    });

    it("a card's own close takes it away and shows the next", () => {
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "first" } });
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "second" } });
        const r = render(() => <PaneRequestDock blockId="a" />);
        fireEvent.click(r.getByText("Done"));
        expect(r.getAllByTestId("card").map((c) => c.textContent)).toEqual([expect.stringContaining("second")]);
        expect(r.queryByTestId("pane-request-more")).toBeNull();
    });
});

describe("PaneRequestModals", () => {
    it("makes every modal it draws operator-only, whatever the caller passed", () => {
        const Probe = (_: PaneRequestProps) => <div data-testid="probe">{String(useContext(OperatorOnlyModals))}</div>;
        openPaneModal("a", Probe, {});
        const r = render(() => <PaneRequestModals blockId="a" />);
        expect(r.getByTestId("probe").textContent).toBe("true");
        const outside = render(() => <Probe close={() => {}} />);
        expect(outside.container.querySelector("[data-testid=probe]")?.textContent).toBe("false");
    });

    it("draws only its pane's modal requests", () => {
        openPaneModal("a", Card, { label: "sign in" });
        openPaneModal("b", Card, { label: "elsewhere" });
        paneRequests.push("a", { source: "vault-use", Component: Card, props: { label: "a card" } });
        const r = render(() => <PaneRequestModals blockId="a" />);
        expect(r.getAllByTestId("card").map((c) => c.textContent)).toEqual([expect.stringContaining("sign in")]);
    });
});
