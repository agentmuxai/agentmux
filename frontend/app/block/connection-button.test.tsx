// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane header's connection chip (`ConnectionButton`): shown on a local
 * pane too, as "Local", so the header is where a pane is switched to a remote
 * and the connection picker always has a chip to open beside.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
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
    it("shows a local pane as Local, and opens the picker from it", () => {
        const { container, ref, opened } = chip(undefined);
        const el = container.querySelector<HTMLDivElement>(".connection-button")!;
        expect(el).toBeTruthy();
        expect(el.classList.contains("connection-button--local")).toBe(true);
        expect(el.textContent).toBe("Local");
        expect(ref.current).toBe(el);
        fireEvent.click(el);
        expect(opened.value).toBe(true);
    });

    it("shows a remote by its nickname, with its colour", () => {
        const { container } = chip("db1");
        const el = container.querySelector<HTMLDivElement>(".connection-button")!;
        expect(el.classList.contains("connection-button--local")).toBe(false);
        expect(el.querySelector(".connection-name")!.textContent).toBe("prod-db");
        expect(el.querySelector<HTMLElement>(".connection-swatch")!.style.background).toBe("rgb(229, 72, 77)");
    });
});
