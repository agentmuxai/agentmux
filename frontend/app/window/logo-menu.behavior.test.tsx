// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * LogoMenu, pinned as a whole ahead of splitting it into a state object and view parts: each scenario mounts the
 * real component with stand-ins for the flyout (which draws the items it is given, with their checks, shortcuts and
 * submenus, and presses them on a click), the stores it reads (the workspaces, the window's theme and opacity, the
 * dev mode, the host's capabilities) and every action a menu item runs, and logs the menu as an indented tree and, in
 * order, every lookup and every call its items make. The logs are snapshots recorded on the unsplit component; the
 * split must leave them byte for byte. (logo-menu.test.tsx asserts what the person gets; this pins the items, their
 * order and the wiring.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, For, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({
    caps: { multiWindow: true, windowTransparency: true } as Record<string, boolean>,
    client: null as any,
    workspaces: null as any,
    theme: null as any,
    opacity: null as any,
    live: null as any,
    label: null as any,
    windowId: null as any,
    dev: null as any,
}));

vi.mock("@/store/global", () => ({
    atoms: { client: () => h.client[0]() },
    getApi: () => ({
        toggleDevtools: () => log("toggleDevtools"),
        openExternal: (url: string) => log(`openExternal ${url}`),
    }),
}));
vi.mock("@/store/tab-actions", () => ({ createTab: () => log("createTab") }));
vi.mock("@/util/host-caps", () => ({ hostHas: (cap: string) => (log(`hostHas ${cap}`), h.caps[cap] ?? false) }));
vi.mock("@/util/util", () => ({
    fireAndForget: (f: () => unknown) => {
        log("fireAndForget");
        Promise.resolve(f()).catch(() => {});
    },
}));
vi.mock("@/app/store/modalmodel", () => ({ openModal: (m: unknown) => log(`openModal ${(m as { name?: string }).name}`) }));
vi.mock("@/app/modals/command-palette", () => ({ CommandPaletteModal: { name: "CommandPaletteModal" } }));
vi.mock("@/app/store/window-appearance", () => ({
    currentWindowLabel: () => h.label[0](),
    currentWindowOpacity: () => h.opacity[0](),
    currentWindowTheme: () => h.theme[0](),
    openNewWindowFromHere: () => (log("openNewWindowFromHere"), Promise.resolve()),
    setCurrentWindowTheme: (id: string) => (log(`setCurrentWindowTheme ${id}`), Promise.resolve()),
}));
vi.mock("@/app/store/window-opacity-store", () => ({
    liveWindowOpacity: (label: string) => h.live[0]()[label],
    setWindowOpacityFor: (target: unknown, value: number, phase: string) => log(`setWindowOpacityFor ${JSON.stringify(target)} ${value} ${phase}`),
}));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => h.windowId[0]() }));
vi.mock("@/app/store/dev-mode", () => ({ devModeOn: () => (log("devModeOn"), h.dev[0]()) }));
vi.mock("@/app/menu/base-menus", () => ({
    THEME_OPTIONS: [
        { id: "dark", label: "Dark" },
        { id: "light", label: "Light" },
    ],
}));
vi.mock("@/app/keybindings", () => ({ shortcutFor: (command: string) => (log(`shortcutFor ${command}`), `<${command}>`) }));
vi.mock("@/app/store/user-workspace", () => ({
    refreshWorkspaceList: () => (log("refreshWorkspaceList"), Promise.resolve()),
    workspaceList: () => h.workspaces[0](),
    workspaceName: (c: { name?: string } | undefined) => c?.name ?? "(none)",
}));
vi.mock("./workspaces", () => ({
    switchWorkspace: (w: { id: string }) => log(`switchWorkspace ${w.id}`),
    newWorkspace: () => log("newWorkspace"),
    importWorkspace: () => log("importWorkspace"),
    exportWorkspace: () => log("exportWorkspace"),
    renameWorkspacePrompt: () => log("renameWorkspacePrompt"),
    deleteWorkspacePrompt: () => log("deleteWorkspacePrompt"),
}));
vi.mock("./window-close", () => ({ requestCloseWindow: () => (log("requestCloseWindow"), Promise.resolve()) }));
vi.mock("./logo-menu.scss", () => ({}));
vi.mock("@/app/element/flyoutmenu", () => ({
    FlyoutMenu: (p: { anchorClass: string; items: any[]; placement: string; mirrored: boolean; closeRef: (fn: () => void) => void; children?: JSX.Element }) => {
        log("FlyoutMenu");
        p.closeRef(() => log("close"));
        const row = (it: any, path: string): JSX.Element => (
            <div
                data-item={path}
                data-label={it.label}
                data-icon={it.icon}
                data-shortcut={it.shortcut}
                data-checked={String(it.checked)}
                data-keepopen={String(it.keepOpen)}
                data-divider={String(!!it.divider)}
                onClick={(e) => {
                    e.stopPropagation();
                    log(`press ${path} ${it.label}`);
                    it.onClick?.();
                }}
            >
                <For each={it.subItems ?? []}>{(sub, i) => row(sub, `${path}.${i()}`)}</For>
            </div>
        );
        return (
            <div data-stand="FlyoutMenu" data-anchor={p.anchorClass} data-placement={p.placement} data-mirrored={String(p.mirrored)}>
                {p.children}
                <For each={p.items}>{(it, i) => row(it, String(i()))}</For>
            </div>
        );
    },
}));

import { closeLogoMenu, LogoMenu } from "./logo-menu";

const SNAP = (name: string) => `./__snapshots__/logo-menu.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|width|height|viewBox|fill|x|y)$/;

/** The menu as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
let unmount: () => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const press = async (path: string) => {
    fireEvent.click(host.querySelector(`[data-item="${path}"]`)!);
    await new Promise((r) => setTimeout(r, 0));
};

function mount(position?: "left" | "right") {
    const view = render(() => <LogoMenu position={position} />);
    host = view.container;
    unmount = view.unmount;
}

beforeEach(() => {
    T.length = 0;
    h.caps = { multiWindow: true, windowTransparency: true };
    h.client = createSignal<any>({ name: "Main", meta: { "workspace:current": "ws-1" } });
    h.workspaces = createSignal<any[]>([]);
    h.theme = createSignal("dark");
    h.opacity = createSignal(1);
    h.live = createSignal<Record<string, number>>({});
    h.label = createSignal<string | undefined>("win-a");
    h.windowId = createSignal("w-1");
    h.dev = createSignal(false);
});
afterEach(() => {
    cleanup();
});

describe("LogoMenu, pinned", () => {
    it("builds its menu in order: the new tab and window, the theme and opacity, the workspaces, the palette, the docs and the exit", () => {
        mount();
        look("left, default host");
        return snapshot("layout");
    });

    it("opens to the right on macOS, and registers its close for the More menu until it unmounts", () => {
        mount("right");
        look("right");
        closeLogoMenu();
        unmount();
        closeLogoMenu();
        log("closed after unmount: nothing above");
        return snapshot("position");
    });

    it("leaves out the window items and the opacity where the host lacks them, and shows DevTools only in dev mode", () => {
        h.caps = { multiWindow: false, windowTransparency: false };
        mount();
        look("a host with neither");
        h.dev[1](true);
        look("dev mode on");
        return snapshot("host-and-dev");
    });

    it("checks the window's theme and the nearest opacity step, following a drag in progress, and sets them without closing", async () => {
        mount();
        look("dark, 100%");
        h.theme[1]("light");
        h.opacity[1](0.82);
        look("light, 80%");
        h.live[1]({ "win-a": 0.52 });
        look("dragging to 50%");
        await press("4.1");
        await press("5.2");
        h.label[1](undefined);
        await press("5.0");
        look("no window label");
        return snapshot("theme-opacity");
    });

    it("lists the workspaces, asks for them again when the open one or its name changes, and runs their actions", async () => {
        mount();
        look("before srv answers");
        h.workspaces[1]([
            { id: "ws-1", name: "Main", current: true },
            { id: "ws-2", name: "Side", current: false },
        ]);
        look("two workspaces");
        h.client[1]({ name: "Main", meta: { "workspace:current": "ws-2" } });
        h.client[1]({ name: "Renamed", meta: { "workspace:current": "ws-2" } });
        await press("6.1.1");
        await press("6.1.0");
        await press("6.0");
        await press("6.2");
        await press("6.3");
        await press("6.5");
        await press("6.6");
        return snapshot("workspaces");
    });

    it("runs the rest of its items: the new tab, the new window, the palette, the devtools, the docs and the exit", async () => {
        h.dev[1](true);
        mount();
        await press("0");
        await press("2");
        await press("8");
        await press("9");
        await press("10");
        await press("12");
        return snapshot("actions");
    });
});
