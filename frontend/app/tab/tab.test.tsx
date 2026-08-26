// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the window tab strip's <Tab> — specifically the close-button
 * click-containment contract from
 * docs/specs/SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §§2-3: a click
 * on the "✕" must fire onClose ONLY, never bubble to the tab's own
 * onClick={onSelect}. The bubbled select is what raced SetActiveTab against
 * CloseTab on the backend and produced the select/deselect flash.
 */

import { cleanup, render } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    atoms: {},
    recordTEvent: vi.fn(),
    refocusNode: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({
    ObjectService: {
        UpdateTabName: vi.fn(() => Promise.resolve()),
        UpdateObjectMeta: vi.fn(() => Promise.resolve()),
    },
}));
vi.mock("@/app/store/wos", () => ({
    makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
    useWaveObjectValue: () => [
        () => ({ otype: "tab", oid: "tab-1", version: 1, name: "Tab One", meta: {} }),
        () => false,
    ],
}));
vi.mock("@/app/tab/tab-measure", () => ({ measureTabWidth: () => 100 }));

import { Tab } from "./tab";

afterEach(() => cleanup());

function renderTab(overrides: { onSelect?: () => void; onClose?: (e: MouseEvent | null) => void } = {}) {
    const onSelect = overrides.onSelect ?? vi.fn();
    const onClose = overrides.onClose ?? vi.fn();
    const utils = render(() => (
        <Tab
            id="tab-1"
            active={false}
            isFirst={false}
            isBeforeActive={false}
            isDragging={false}
            tabWidth={0}
            isNew={false}
            onSelect={onSelect}
            onClose={onClose}
            onDragStart={vi.fn()}
            onLoaded={vi.fn()}
        />
    ));
    return { ...utils, onSelect, onClose };
}

describe("Tab close button", () => {
    it("fires onClose and never onSelect when the close button of a background tab is clicked", async () => {
        const { container, onSelect, onClose } = renderTab();
        const closeButton = container.querySelector<HTMLButtonElement>("[title='Close Tab']");
        expect(closeButton).not.toBeNull();

        await userEvent.click(closeButton!);

        expect(onClose).toHaveBeenCalledTimes(1);
        expect(onSelect).not.toHaveBeenCalled();
    });

    it("still fires onSelect for a click on the tab body itself", async () => {
        const { container, onSelect, onClose } = renderTab();
        const tabEl = container.querySelector<HTMLDivElement>(".tab");
        expect(tabEl).not.toBeNull();

        await userEvent.click(tabEl!.querySelector(".name")!);

        expect(onSelect).toHaveBeenCalledTimes(1);
        expect(onClose).not.toHaveBeenCalled();
    });
});
