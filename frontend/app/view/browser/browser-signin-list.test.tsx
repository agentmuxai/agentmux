// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The list of saved sign-ins under a login field: what srv writes to `browser:signin-list` is drawn as
 * rows, a pick is the person's click, the selected row follows srv's cursor, and the panel is marked
 * operator-only so no agent's UI click or key press reaches it
 * (docs/specs/SPEC_VAULT_AUTOFILL_IN_BROWSER_PANE_2026_11_14.md §P2.3, §P2.4).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const fill = vi.fn(() => Promise.resolve());
const strong = vi.fn(() => Promise.resolve());
const close = vi.fn(() => Promise.resolve());
const openVault = vi.fn(() => Promise.resolve());
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SigninFillCommand: (...a: unknown[]) => fill(...(a as [])),
        SignupPasswordCommand: (...a: unknown[]) => strong(...(a as [])),
        SigninListCloseCommand: (...a: unknown[]) => close(...(a as [])),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// The host's `browser-pane-clicked` events, which the test fires itself.
const paneClicks = new Set<(p: { block_id: string }) => void>();
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        listen: (_name: string, cb: (p: { block_id: string }) => void) => {
            paneClicks.add(cb);
            return Promise.resolve(() => paneClicks.delete(cb));
        },
    }),
}));
// jsdom marks every script-dispatched event untrusted and can't be told otherwise, so the document's
// pointerdown listeners are kept here and called with an event that says it was the person's own.
const presses = new Set<(e: Event) => void>();
const realAdd = document.addEventListener.bind(document);
const realRemove = document.removeEventListener.bind(document);
document.addEventListener = ((type: string, fn: EventListenerOrEventListenerObject, opts?: boolean | AddEventListenerOptions) => {
    if (type === "pointerdown" && typeof fn === "function") presses.add(fn as (e: Event) => void);
    realAdd(type, fn, opts);
}) as typeof document.addEventListener;
document.removeEventListener = ((type: string, fn: EventListenerOrEventListenerObject, opts?: boolean | EventListenerOptions) => {
    if (type === "pointerdown" && typeof fn === "function") presses.delete(fn as (e: Event) => void);
    realRemove(type, fn, opts);
}) as typeof document.removeEventListener;
/** A press the person made on `el`. */
const press = (el: Element) => presses.forEach((h) => h({ isTrusted: true, target: el } as unknown as Event));
vi.mock("@/app/view/vault/open-vault", () => ({ openVault: (...a: unknown[]) => openVault(...(a as [])) }));
// The popover is tested on its own; here only what the list hands it.
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (p: { children: unknown; ref?: (el: HTMLElement) => void; role?: string; class?: string; onShown?: unknown; "aria-label"?: string }) => (
        <div ref={(el) => p.ref?.(el)} role={p.role as never} class={p.class} aria-label={p["aria-label"]} data-has-onshown={p.onShown ? "yes" : "no"}>
            {p.children as never}
        </div>
    ),
}));

import { BrowserSigninList } from "./browser-signin-list";
import { listRowLabel, readSigninList } from "./browser-signin-offer-model";

afterEach(() => {
    cleanup();
    fill.mockClear();
    strong.mockClear();
    close.mockClear();
    openVault.mockClear();
});

const list = (over: Record<string, unknown> = {}) => ({
    "browser:signin-list": {
        origin: "https://accounts.example.com",
        role: "password",
        rows: [
            { entry: "e1", user: "al***" },
            { entry: "e2", user: "" },
        ],
        strong: false,
        cursor: null,
        ...over,
    },
});

describe("BrowserSigninList", () => {
    it("draws nothing when srv has no list open", () => {
        const r = render(() => <BrowserSigninList blockId="b1" meta={() => ({})} />);
        expect(r.queryByTestId("browser-signin-list")).toBeNull();
    });

    it("draws the masked user names and a Manage passwords row, never taking focus", async () => {
        const r = render(() => (
            <div>
                <BrowserSigninList blockId="b1" meta={() => list()} />
            </div>
        ));
        const panel = await r.findByTestId("browser-signin-list");
        expect(panel.hasAttribute("data-operator-only")).toBe(true);
        expect(panel.getAttribute("data-has-onshown")).toBe("no");
        expect(r.getAllByTestId("browser-signin-list-row").map((x) => x.textContent)).toEqual(["al***", "No user name"]);
        expect(r.getByTestId("browser-signin-list-manage").textContent).toBe("Manage passwords");
        // A press on the panel doesn't move the page field's focus.
        const press = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
        panel.dispatchEvent(press);
        expect(press.defaultPrevented).toBe(true);
    });

    it("marks the row srv selected", async () => {
        const r = render(() => <BrowserSigninList blockId="b1" meta={() => list({ cursor: 1 })} />);
        await r.findByTestId("browser-signin-list");
        const rows = r.getAllByTestId("browser-signin-list-row");
        expect(rows.map((x) => x.getAttribute("aria-selected"))).toEqual(["false", "true"]);
        expect(rows[1].classList.contains("active")).toBe(true);
    });

    it("fills the clicked sign-in", async () => {
        const r = render(() => <BrowserSigninList blockId="b1" meta={() => list()} />);
        await r.findByTestId("browser-signin-list");
        fireEvent.click(r.getAllByTestId("browser-signin-list-row")[0]);
        expect(fill).toHaveBeenCalledWith({}, { block_id: "b1", entry: "e1" });
    });

    it("offers only a strong password in a field for a new password, and makes one on a click", async () => {
        const r = render(() => <BrowserSigninList blockId="b1" meta={() => list({ role: "newpassword", rows: [], strong: true })} />);
        await r.findByTestId("browser-signin-list");
        expect(r.queryAllByTestId("browser-signin-list-row")).toHaveLength(0);
        fireEvent.click(r.getByTestId("browser-signin-list-strong"));
        expect(strong).toHaveBeenCalledWith({}, { block_id: "b1" });
        expect(fill).not.toHaveBeenCalled();
    });

    it("opens the Vault on its logins from Manage passwords", async () => {
        const r = render(() => <BrowserSigninList blockId="b1" meta={() => list()} />);
        await r.findByTestId("browser-signin-list");
        fireEvent.click(r.getByTestId("browser-signin-list-manage"));
        expect(openVault).toHaveBeenCalledWith("logins");
    });

    it("closes on a press elsewhere in the app's UI, not on a press on the panel", async () => {
        const [meta, setMeta] = createSignal<Record<string, unknown>>(list());
        const outside = document.createElement("button");
        document.body.appendChild(outside);
        const r = render(() => <BrowserSigninList blockId="b1" meta={meta} />);
        const panel = await r.findByTestId("browser-signin-list");
        press(panel);
        expect(close).not.toHaveBeenCalled();
        press(outside);
        expect(close).toHaveBeenCalledWith({}, { block_id: "b1" });
        // srv closed it: nothing listens any more.
        setMeta({});
        await Promise.resolve();
        await Promise.resolve();
        close.mockClear();
        press(outside);
        paneClicks.forEach((cb) => cb({ block_id: "b2" }));
        expect(close).not.toHaveBeenCalled();
        outside.remove();
    });

    it("takes a click in the page of this same pane, which the DOM sees as a synthetic press, for the page's own", async () => {
        const r = render(() => <BrowserSigninList blockId="b1" meta={() => list()} />);
        await r.findByTestId("browser-signin-list");
        // BrowserPaneOutsideClickBridge: an untrusted press on the body, and the host's event naming the pane.
        document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
        paneClicks.forEach((cb) => cb({ block_id: "b1" }));
        expect(close).not.toHaveBeenCalled();
    });

    it("closes on a click in the page of another browser pane", async () => {
        const r = render(() => <BrowserSigninList blockId="b1" meta={() => list()} />);
        await r.findByTestId("browser-signin-list");
        paneClicks.forEach((cb) => cb({ block_id: "b2" }));
        expect(close).toHaveBeenCalledWith({}, { block_id: "b1" });
    });

    it("says why when a fill fails", async () => {
        fill.mockImplementationOnce(() => Promise.reject(new Error("the page changed")));
        const r = render(() => <BrowserSigninList blockId="b1" meta={() => list()} />);
        await r.findByTestId("browser-signin-list");
        fireEvent.click(r.getAllByTestId("browser-signin-list-row")[0]);
        expect(await r.findByText("the page changed")).toBeTruthy();
    });
});

describe("readSigninList", () => {
    it("reads what srv writes and nothing else", () => {
        expect(readSigninList(list())?.rows.map((x) => x.entry)).toEqual(["e1", "e2"]);
        expect(readSigninList(list({ cursor: 1 }))?.cursor).toBe(1);
        expect(readSigninList(list({ role: "elsewhere" }))?.role).toBe("username");
    });

    it("is no list for null, a list with nothing to show, or a malformed one", () => {
        expect(readSigninList(undefined)).toBeUndefined();
        expect(readSigninList({ "browser:signin-list": null })).toBeUndefined();
        expect(readSigninList(list({ rows: [] }))).toBeUndefined();
        expect(readSigninList(list({ rows: [null, { user: "x" }] }))).toBeUndefined();
        expect(readSigninList({ "browser:signin-list": { origin: 3, rows: [] } })).toBeUndefined();
    });

    it("keeps a strong-password-only list", () => {
        expect(readSigninList(list({ rows: [], strong: true }))?.strong).toBe(true);
    });

    it("names a row with no user name", () => {
        expect(listRowLabel({ user: "" })).toBe("No user name");
        expect(listRowLabel({ user: "al***" })).toBe("al***");
    });
});
