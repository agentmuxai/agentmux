// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane header's connection chip (`ConnectionButton`): shown on a remote
 * pane, by the remote's name. A local pane has none; Change connection
 * (⌘⇧G) still opens the picker there, placed by the pane.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/block/blockframe", () => ({ NumActiveConnColors: 8 }));
vi.mock("@/app/block/pane-tab-registry", () => ({ paneTabIconFor: () => undefined, paneTabLabelFor: () => undefined }));
vi.mock("@/app/store/global", () => ({
    atoms: { fullConfigAtom: () => ({ connections: { db1: { "display:name": "prod-db", "display:color": "#e5484d" } } }) },
    getConnStatusAtom: () => () => ({ status: "connected", connected: true, activeconnnum: 1 }),
}));
vi.mock("../asset/dots-anim-4.svg?url", () => ({ default: "" }));

import { ConnectionButton } from "./blockutil";

afterEach(() => cleanup());

function chip(connection: string | undefined) {
    const opened = { value: false };
    const atom = Object.assign(() => opened.value, { _set: (v: boolean) => (opened.value = v) }) as any;
    const ref = { current: null as HTMLDivElement | null };
    const utils = render(() => <ConnectionButton ref={ref} connection={connection} changeConnModalAtom={atom} />);
    return { ...utils, ref, opened };
}

describe("ConnectionButton", () => {
    it("shows nothing on a local pane", () => {
        for (const local of [undefined, "", "local"]) {
            const { container, ref, unmount } = chip(local);
            expect(container.querySelector(".connection-button")).toBeNull();
            expect(container.textContent).toBe("");
            expect(ref.current).toBeNull();
            unmount();
        }
    });

    it("clears the picker's anchor when a remote pane becomes local", () => {
        // A stale anchor (the removed chip) would defeat the picker's
        // fallback to the pane, and Change connection would open misplaced.
        const [conn, setConn] = createSignal<string | undefined>("db1");
        const ref = { current: null as HTMLDivElement | null };
        const atom = Object.assign(() => false, { _set: () => {} }) as any;
        const { container } = render(() => <ConnectionButton ref={ref} connection={conn()} changeConnModalAtom={atom} />);
        expect(ref.current).toBe(container.querySelector(".connection-button"));
        setConn("");
        expect(container.querySelector(".connection-button")).toBeNull();
        expect(ref.current).toBeNull();
    });

    it("shows a remote by its nickname, with its colour, and opens the picker from it", () => {
        const { container, ref, opened } = chip("db1");
        const el = container.querySelector<HTMLDivElement>(".connection-button")!;
        expect(ref.current).toBe(el);
        expect(el.querySelector(".connection-name")!.textContent).toBe("prod-db");
        expect(el.querySelector<HTMLElement>(".connection-swatch")!.style.background).toBe("rgb(229, 72, 77)");
        fireEvent.click(el);
        expect(opened.value).toBe(true);
    });
});
