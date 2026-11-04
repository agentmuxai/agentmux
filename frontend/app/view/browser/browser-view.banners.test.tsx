// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The banners a browser pane draws over its page, pinned as markup: the attention request in each of
 * its kinds (and what each button sends), the popup-window list, and the "popup from" note. The real
 * BrowserViewComponent is mounted over a stand-in model; the native-pane hooks and the strips around
 * the banners are stubbed, because this is about what the banners look like. A refactor of the view
 * must leave the committed snapshots as they are; regenerate with `vitest -u` only for a change meant
 * to look different.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTestHostApi } from "@/app/host/test-host";

vi.mock("@/element/modal-layer", () => ({ useModalLayer: () => ({}) }));
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: () => () => "active" }));
vi.mock("@/app/workspace/window-tab-visibility", () => ({ useWindowTabId: () => "t1" }));
vi.mock("@/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: () => {} }));
vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));
vi.mock("./use-pane-rect-sync", () => ({
    usePaneRectSync: () => ({ paneCreated: () => true, createPane: async () => {}, paneRect: () => null }),
}));
vi.mock("./use-freeze-frame", () => ({ useFreezeFrame: () => ({ freezeSnapshot: () => null, freezeStyle: () => ({}) }) }));
vi.mock("./use-drag-snapshot", () => ({ useDragSnapshot: () => ({ take: async () => null }) }));
vi.mock("./use-browser-auth", () => ({ useBrowserAuth: () => {} }));
vi.mock("./browser-nav-bar", () => ({ BrowserNavBar: () => <div class="stub-nav" /> }));
vi.mock("./browser-signin-offer", () => ({ BrowserSigninOffer: () => null, BrowserSigninPending: () => null }));
vi.mock("./download-strip", () => ({ DownloadStrip: () => null }));
vi.mock("./driven-by-strip", () => ({ ActionLogStrip: () => null, DrivenByStrip: () => null }));
vi.mock("@/app/drag/drag-session", () => ({ isAnyUnderway: () => false }));
vi.mock("@/app/element/BrainSpinner", () => ({ BrainSpinner: () => null }));

import { BrowserViewComponent } from "./browser-view";

type Attention = BrowserAttention;
const att = (over: Partial<Attention>): Attention => ({ id: "a1", kind: "handoff", ...over }) as Attention;

function mount(init: { attention?: Attention; popups?: { id: string; url: string }[]; popupFrom?: string; canGoBack?: boolean } = {}) {
    const [attention, setAttention] = createSignal<Attention | undefined>(init.attention);
    const [popups, setPopups] = createSignal(init.popups ?? []);
    const [popupFrom, setPopupFrom] = createSignal<string | undefined>(init.popupFrom);
    const calls: string[] = [];
    const call = (name: string) => async (...a: unknown[]) => {
        calls.push([name, ...a.map(String)].join(" "));
    };
    const atoms: Record<string, () => unknown> = {
        urlAtom: () => "https://example.com/",
        loadingAtom: () => false,
        sleepAtom: () => undefined,
        keepAwakeAtom: () => false,
        driverAgentAtom: () => undefined,
        bypassAtom: () => undefined,
        errorAtom: () => undefined,
        actionsAtom: () => ({ count: 0, entries: [] }),
        canGoBackAtom: () => init.canGoBack ?? true,
        attentionAtom: attention,
        popupWindowsAtom: popups,
        popupFromAtom: popupFrom,
        meta: () => ({}),
    };
    const model = new Proxy(
        { blockId: "blk-1234567", closed: false, resolveAttention: call("resolve"), showPopup: call("show"), closePopup: call("close") } as Record<string, unknown>,
        { get: (t, k: string) => (k in t ? t[k] : k in atoms ? atoms[k] : () => undefined) },
    );
    const r = render(() => <BrowserViewComponent model={model as never} />);
    const banner = () => r.container.querySelector(".browser-attention");
    return { ...r, calls, setAttention, setPopups, setPopupFrom, banner };
}

/** One element per line, so a snapshot diff points at the element that changed. */
const markup = (el: Element | null): string => (el ? el.outerHTML : "(absent)").replace(/></g, ">\n<") + "\n";
const snap = (name: string) => `./__snapshots__/browser-view.banners.${name}.html`;

beforeEach(() => {
    window.api = makeTestHostApi({}, { nativeBrowserPane: true });
});
afterEach(() => cleanup());

describe("the attention banner", () => {
    const kinds: [string, Attention][] = [
        ["handoff", att({ kind: "handoff", agent: "Korp", reason: "sign in to the portal" })],
        ["navigation", att({ kind: "navigation", origin: "https://other.example", url: "https://other.example/x" })],
        ["navigation-popup", att({ kind: "navigation", origin: "https://other.example", url: "https://other.example/x", popup: true })],
        ["notice", att({ kind: "notice", url: "https://blocked.example/y", origin: "https://blocked.example", sites: ["https://a.example", "https://b.example"], why: "It is not on the list.", allow: true })],
        ["bypass", att({ kind: "bypass", agent: "Korp", origins: ["https://a.example", "https://b.example"], minutes: 90 })],
        ["bypass-terminal", att({ kind: "bypass", agent: "Korp", origins: ["https://a.example"], minutes: 60, powers: ["terminal"], local: false })],
        ["dialog-alert", att({ kind: "dialog", dialogKind: "alert", origin: "https://a.example", message: "Saved!" })],
        ["dialog-leave", att({ kind: "dialog", dialogKind: "beforeunload", origin: "https://a.example", agent: "Korp", message: "Leave this page?" })],
        [
            "approval",
            att({
                kind: "approval",
                agent: "Korp",
                what: 'Click "Submit"',
                description: "x".repeat(230),
                why: "it submits a form",
                action: "https://a.example/post",
                fields: [["email", "a@b.c"], ["password", "••••"]],
                window: "https://pop.example/",
            }),
        ],
    ];

    it.each(kinds)("draws a %s request", async (name, a) => {
        const v = mount({ attention: a });
        await expect(markup(v.banner())).toMatchFileSnapshot(snap(name));
    });

    it("sends the answer each button stands for", () => {
        const v = mount({ attention: att({ kind: "bypass", agent: "Korp", origins: ["https://a.example"], minutes: 60 }) });
        const buttons = [...v.container.querySelectorAll(".browser-attention-actions button")];
        expect(buttons.map((b) => b.textContent?.trim())).toEqual(["Allow auto-approve", "Always for Korp on this site", "No"]);
        for (const b of buttons) fireEvent.click(b);
        expect(v.calls).toEqual(["resolve approve", "resolve approve_always", "resolve cancel"]);
    });

    it("a notice offers Back, which the model can refuse", () => {
        const v = mount({ attention: att({ kind: "notice", url: "https://x.example", why: "no" }), canGoBack: false });
        const back = [...v.container.querySelectorAll(".browser-attention-actions button")].find((b) => b.textContent?.trim() === "Back") as HTMLButtonElement;
        expect(back.disabled).toBe(true);
    });

    it("is gone when nothing is asked", () => {
        const v = mount({ attention: att({ kind: "handoff", reason: "x" }) });
        expect(v.banner()).not.toBeNull();
        v.setAttention(undefined);
        expect(v.banner()).toBeNull();
    });
});

describe("popup windows", () => {
    it("lists each popup with Show and Close, and says where one came from", async () => {
        const v = mount({ popups: [{ id: "p1", url: "https://pop.example/a" }, { id: "p2", url: "https://pop.example/b" }], popupFrom: "https://opener.example" });
        await expect(markup(v.container.querySelector(".browser-popup-windows"))).toMatchFileSnapshot(snap("popup-list"));
        await expect(markup(v.container.querySelector(".browser-popup-from"))).toMatchFileSnapshot(snap("popup-from"));
        const buttons = [...v.container.querySelectorAll(".browser-popup-window button")];
        fireEvent.click(buttons[0]);
        fireEvent.click(buttons[3]);
        expect(v.calls).toEqual(["show p1", "close p2"]);
    });

    it("draws neither with no popups", () => {
        const v = mount();
        expect(v.container.querySelector(".browser-popup-windows")).toBeNull();
        expect(v.container.querySelector(".browser-popup-from")).toBeNull();
    });
});
