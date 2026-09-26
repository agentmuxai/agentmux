// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dropping files on an agent pane needs real local paths, so it is wired
 * only on a host with HostCaps.nativeFileDrop (was: `detectHost() === "cef"`).
 * docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md §5, slice 5.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: () => () => undefined,
    pushNotification: vi.fn(),
    MOS: { getObjectValue: () => undefined, makeORef: (t: string, id: string) => `${t}:${id}` },
}));
vi.mock("@/util/dnd", () => ({
    baseName: (p: string) => p,
    consumeDragPaths: vi.fn(() => Promise.resolve([])),
    copyFilesToDir: vi.fn(),
}));

import { CEF_HOST_CAPS } from "@/app/host/host-caps";
import { makeTestHostApi } from "@/app/host/test-host";
import { useAgentDropAttach } from "./useAgentDropAttach";

afterEach(() => cleanup());

function fileDragOver(el: HTMLElement): void {
    const e = new Event("dragover", { bubbles: true, cancelable: true });
    Object.defineProperty(e, "dataTransfer", { value: { types: ["Files"] } });
    el.dispatchEvent(e);
}

function mountAndDrag(): boolean {
    let root!: HTMLDivElement;
    let result!: ReturnType<typeof useAgentDropAttach>;
    function Harness() {
        result = useAgentDropAttach({ blockId: "b1", rootRef: () => root } as Parameters<typeof useAgentDropAttach>[0]);
        return <div ref={root} />;
    }
    render(() => <Harness />);
    fileDragOver(root);
    return result.isDragOver();
}

describe("agent pane file drop follows nativeFileDrop", () => {
    it("is not wired on a host without it", () => {
        window.api = makeTestHostApi();
        expect(mountAndDrag()).toBe(false);
    });

    it("is wired on the CEF host", () => {
        window.api = makeTestHostApi({}, CEF_HOST_CAPS);
        expect(mountAndDrag()).toBe(true);
    });
});
