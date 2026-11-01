// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The key button's menu: its pick is the person's approval to sign in, so it is
 * marked operator-only, which agents' UI query, click and key presses refuse
 * (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.3).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const fill = vi.fn(() => Promise.resolve());
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SigninFillCommand: (...a: unknown[]) => fill(...(a as [])) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// The kit's Menu is tested on its own (Menu.test.tsx, operator-only included);
// here only what the key button hands it.
vi.mock("@/app/element/ui", async (original) => {
    const ui = await original<typeof import("@/app/element/ui")>();
    const { For } = await import("solid-js");
    return {
        ...ui,
        Menu: (p: { items: { label: string; onSelect?: () => void }[]; operatorOnly?: boolean; onClose: () => void }) => (
            <div role="menu" data-operator-only={p.operatorOnly ? "" : undefined}>
                <For each={p.items}>
                    {(i) => (
                        <div role="menuitem" onClick={() => (i.onSelect?.(), p.onClose())}>
                            {i.label}
                        </div>
                    )}
                </For>
            </div>
        ),
    };
});

import { BrowserSigninKey } from "./browser-signin-key";

afterEach(() => {
    cleanup();
    fill.mockClear();
});

const meta = () => ({
    "browser:signins": [
        { entry: "e1", origin: "https://accounts.example.com", user: "al***" },
        { entry: "e2", origin: "https://accounts.example.com", user: "sa***" },
    ],
});

describe("BrowserSigninKey", () => {
    it("shows nothing when the page has no saved sign-in", () => {
        const r = render(() => <BrowserSigninKey blockId="b1" meta={() => ({})} />);
        expect(r.queryByTestId("browser-signin-key")).toBeNull();
    });

    it("opens an operator-only menu whose pick fills that sign-in", async () => {
        const r = render(() => <BrowserSigninKey blockId="b1" meta={meta} />);
        fireEvent.click(r.getByTestId("browser-signin-key"));
        const menu = await r.findByRole("menu");
        expect(menu.closest("[data-operator-only]")).not.toBeNull();
        const items = r.getAllByRole("menuitem");
        expect(items.map((i) => i.textContent)).toEqual([expect.stringContaining("al***"), expect.stringContaining("sa***")]);
        fireEvent.click(items[1]);
        expect(fill).toHaveBeenCalledWith({}, { block_id: "b1", entry: "e2" });
    });
});
