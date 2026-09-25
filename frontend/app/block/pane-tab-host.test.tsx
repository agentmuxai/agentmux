// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { render } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const [blockSig, setBlockSig] = createSignal<{ meta?: Record<string, unknown> }>({ meta: { "x:y": 1 } });
vi.mock("@/store/mos", () => ({
    getMuxObjectAtom: () => blockSig,
    makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
}));
const setMeta = vi.fn((..._args: unknown[]) => Promise.resolve());
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetMetaCommand: (...a: unknown[]) => setMeta(...a) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: "tab-rpc" }));
const [dormantSig, setDormant] = createSignal(false);
vi.mock("@/app/store/block-component-registry", () => ({ isBlockDormant: () => dormantSig }));
vi.mock("@/app/workspace/window-tab-visibility", () => ({ useWindowTabDisplayed: () => () => true }));

import { adaptPaneTabInstance, makePaneTabHostContext } from "./pane-tab-host";
import type { PaneTabInstance, PaneTabManifest } from "./pane-tab-registry";

const manifest: PaneTabManifest = {
    apiVersion: 1,
    view: "native",
    label: "Native",
    icon: "n",
    create: () => ({ component: () => null as any }),
};

afterEach(() => {
    setMeta.mockClear();
    setBlockSig({ meta: { "x:y": 1 } });
});

describe("PaneTabHostContext", () => {
    it("exposes the block id and its meta, reactively", () => {
        createRoot((dispose) => {
            const ctx = makePaneTabHostContext("b1", { isFocused: () => true } as any);
            expect(ctx.blockId).toBe("b1");
            expect(ctx.meta()?.["x:y"]).toBe(1);
            setBlockSig({ meta: { "x:y": 2 } });
            expect(ctx.meta()?.["x:y"]).toBe(2);
            expect(ctx.isFocused()).toBe(true);
            dispose();
        });
    });

    it("exposes the tab's one visibility signal", () => {
        createRoot((dispose) => {
            const ctx = makePaneTabHostContext("b1", {} as any);
            expect(ctx.visibility()).toBe("active");
            setDormant(true);
            expect(ctx.visibility()).toBe("dormant");
            setDormant(false);
            dispose();
        });
    });

    it("setMeta writes the block's meta through the RPC", async () => {
        const ctx = makePaneTabHostContext("b1", {} as any);
        await ctx.setMeta({ "help:zoom": 1.1 });
        expect(setMeta).toHaveBeenCalledWith("tab-rpc", { oref: "block:b1", meta: { "help:zoom": 1.1 } });
        expect(ctx.isFocused()).toBe(false);
    });
});

describe("adaptPaneTabInstance", () => {
    it("presents a native instance as the ViewModel the host consumes", () => {
        const dispose = vi.fn();
        const focus = vi.fn(() => true);
        const instance: PaneTabInstance = {
            component: (p) => <div data-testid="native">{p.ctx.blockId}</div>,
            liveTitle: () => ({ text: "Page", placeholder: true }),
            liveFavicon: () => "fav.ico",
            focus,
            dispose,
        };
        const ctx = makePaneTabHostContext("b1", {} as any);
        const vm = adaptPaneTabInstance(manifest, ctx, instance);

        expect(vm.viewType).toBe("native");
        expect(vm.blockId).toBe("b1");
        expect(vm.viewName?.()).toBe("Page");
        expect(vm.viewNameIsPlaceholder?.()).toBe(true);
        expect(vm.viewFaviconUrl?.()).toBe("fav.ico");
        expect(vm.giveFocus?.()).toBe(true);
        vm.dispose?.();
        expect(dispose).toHaveBeenCalledOnce();

        const VC = vm.viewComponent;
        const { getByTestId } = render(() => (
            <VC blockId="b1" blockRef={{ current: null }} contentRef={{ current: null }} model={vm} />
        ));
        expect(getByTestId("native").textContent).toBe("b1");
    });

    it("maps the settings menu, and the connection / noPadding capabilities", () => {
        const menu = [{ label: "Plot Type" }];
        const vm = adaptPaneTabInstance(
            { ...manifest, capabilities: { connection: true, noPadding: true } },
            makePaneTabHostContext("b1", {} as any),
            { component: () => null as any, settingsMenu: () => menu as any }
        );
        expect(vm.getSettingsMenuItems?.()).toBe(menu);
        expect(vm.manageConnection?.()).toBe(true);
        expect(vm.noPadding?.()).toBe(true);
    });

    it("maps a live header icon", () => {
        const icon = { elemtype: "iconbutton", icon: "file-code" } as any;
        const vm = adaptPaneTabInstance(manifest, makePaneTabHostContext("b1", {} as any), {
            component: () => null as any,
            headerIcon: () => icon,
        });
        expect(vm.viewIcon?.()).toBe(icon);
    });

    it("leaves out what the instance does not provide, so the host's defaults apply", () => {
        const vm = adaptPaneTabInstance(manifest, makePaneTabHostContext("b1", {} as any), {
            component: () => null as any,
        });
        expect(vm.viewName).toBeUndefined();
        expect(vm.viewIcon).toBeUndefined();
        expect(vm.giveFocus).toBeUndefined();
        expect(vm.getBodyContextMenuItems).toBeUndefined();
        expect(vm.getSettingsMenuItems).toBeUndefined();
        expect(vm.manageConnection).toBeUndefined();
        expect(vm.noPadding).toBeUndefined();
    });
});
